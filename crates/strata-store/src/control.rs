//! SQLite 控制存储。`ControlStore` 是后端界面，本阶段没有 PostgreSQL 连接。

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use strata_model::{
    AccessTask, ChangeSet, CommitId, CommitResult, QueueName, TaskId, Timestamp, WaitFor,
};

use crate::error::Error;
use crate::resolve::{self, HeadSet, Resolved};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS outbox (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  commit_id TEXT NOT NULL UNIQUE,
  idempotency_key TEXT UNIQUE,
  payload TEXT NOT NULL,
  status TEXT NOT NULL,
  snapshot_seq INTEGER,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS identity_assign (
  identity_key TEXT PRIMARY KEY,
  rid TEXT NOT NULL,
  scheme TEXT NOT NULL,
  source_id TEXT,
  tombstone INTEGER NOT NULL,
  commit_id TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS resource_head (
  rid TEXT PRIMARY KEY,
  ns TEXT NOT NULL,
  source_id TEXT,
  created_at TEXT NOT NULL,
  tombstone INTEGER NOT NULL,
  commit_id TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS state_head (
  scope TEXT NOT NULL,
  key TEXT NOT NULL,
  version INTEGER NOT NULL,
  value TEXT NOT NULL,
  commit_id TEXT NOT NULL,
  PRIMARY KEY (scope, key)
);
CREATE TABLE IF NOT EXISTS staging_ref (
  sha256 TEXT NOT NULL,
  commit_id TEXT NOT NULL,
  PRIMARY KEY (sha256, commit_id)
);
CREATE TABLE IF NOT EXISTS tasks (
  task_id TEXT PRIMARY KEY,
  queue TEXT NOT NULL,
  payload TEXT NOT NULL,
  status TEXT NOT NULL,
  lease_until TEXT,
  retry_at TEXT,
  last_error TEXT,
  priority INTEGER NOT NULL,
  created_at TEXT NOT NULL,
  source_id TEXT NOT NULL DEFAULT '',
  locator_key TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS committer_lease (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  holder TEXT,
  until TEXT
);
CREATE TABLE IF NOT EXISTS hot (
  kind TEXT NOT NULL,
  key TEXT NOT NULL,
  value TEXT NOT NULL,
  PRIMARY KEY (kind, key)
);
CREATE INDEX IF NOT EXISTS tasks_queue ON tasks(queue, status);
CREATE INDEX IF NOT EXISTS tasks_locator ON tasks(source_id, locator_key, status);
";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct FlushBatch {
    pub commit_ids: Vec<String>,
    pub versions: std::collections::BTreeMap<String, u64>,
}

pub(crate) struct SqliteControl {
    pool: SqlitePool,
}

pub(crate) trait ControlStore {
    async fn accept(
        &self,
        changeset: &ChangeSet,
        commit_id: &CommitId,
        now: Timestamp,
    ) -> Result<CommitResult, Error>;

    async fn list_accepted(&self) -> Result<Vec<resolve::StoredCommit>, Error>;

    async fn flush_batch(&self) -> Result<Option<FlushBatch>, Error>;

    async fn put_flush_batch(&self, batch: &FlushBatch) -> Result<(), Error>;

    async fn finish_visible(&self, commit_ids: &[String], snapshot: u64) -> Result<(), Error>;

    async fn clear_flush_batch(&self) -> Result<(), Error>;

    async fn baseline(&self, table: &str) -> Result<Option<u64>, Error>;

    async fn set_baseline(&self, table: &str, version: u64) -> Result<(), Error>;

    async fn alloc_seq(&self, count: u64) -> Result<u64, Error>;

    async fn alloc_snapshot(&self) -> Result<u64, Error>;

    async fn enqueue(&self, tasks: &[AccessTask], now: Timestamp) -> Result<(), Error>;

    async fn lease(
        &self,
        queue: QueueName,
        count: usize,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<Vec<AccessTask>, Error>;

    async fn extend(
        &self,
        task_id: &TaskId,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<(), Error>;

    async fn nack(&self, task_id: &TaskId, retry_at: Timestamp, error: &str) -> Result<(), Error>;

    async fn lease_id(
        &self,
        task_id: &TaskId,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<AccessTask, Error>;

    async fn release_leases(&self) -> Result<Vec<AccessTask>, Error>;

    async fn leased_count(&self, queue: QueueName, now: Timestamp) -> Result<usize, Error>;

    async fn task_active(&self, source: &str, locator_key: &str) -> Result<bool, Error>;

    async fn lease_ready(
        &self,
        source: &str,
        locator_key: &str,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<Option<AccessTask>, Error>;

    async fn accepted_state(&self, scope: &str, key: &str) -> Result<Option<(u64, String)>, Error>;

    async fn hot_get(&self, kind: &str, key: &str) -> Result<Option<String>, Error>;

    async fn hot_put(&self, kind: &str, key: &str, value: &str) -> Result<(), Error>;

    async fn staging_for(&self, commit_ids: &[String]) -> Result<Vec<String>, Error>;

    async fn accepted_staging(&self) -> Result<HashSet<String>, Error>;

    async fn outcome(&self, commit_id: &str) -> Result<Option<CommitResult>, Error>;
}

impl SqliteControl {
    pub(crate) async fn open(path: &Path) -> Result<Self, Error> {
        let url = format!("sqlite://{}?mode=rwc", path.display());
        let pool = SqlitePool::connect(&url)
            .await
            .map_err(|source| Error::Sql {
                context: "打开控制库",
                source,
            })?;
        sqlx::query("PRAGMA journal_mode = WAL")
            .fetch_optional(&pool)
            .await
            .map_err(|source| Error::Sql {
                context: "设置 WAL",
                source,
            })?;
        sqlx::query("PRAGMA synchronous = NORMAL")
            .execute(&pool)
            .await
            .map_err(|source| Error::Sql {
                context: "设置 synchronous",
                source,
            })?;
        sqlx::query("PRAGMA busy_timeout = 5000")
            .execute(&pool)
            .await
            .map_err(|source| Error::Sql {
                context: "设置 busy_timeout",
                source,
            })?;
        for statement in SCHEMA.split(';') {
            let statement = statement.trim();
            if statement.is_empty() {
                continue;
            }
            sqlx::query(statement)
                .execute(&pool)
                .await
                .map_err(|source| Error::Sql {
                    context: "初始化控制库",
                    source,
                })?;
        }
        ensure_task_column(&pool, "source_id", "TEXT NOT NULL DEFAULT ''").await?;
        ensure_task_column(&pool, "locator_key", "TEXT NOT NULL DEFAULT ''").await?;
        Ok(Self { pool })
    }
}

impl ControlStore for SqliteControl {
    async fn accept(
        &self,
        changeset: &ChangeSet,
        commit_id: &CommitId,
        now: Timestamp,
    ) -> Result<CommitResult, Error> {
        let now_text = now.to_rfc3339()?;
        let mut tx = self.pool.begin().await.map_err(|source| Error::Sql {
            context: "开始接收事务",
            source,
        })?;
        if let Some(key) = &changeset.idempotency_key {
            if let Some(existing) = duplicate(&mut tx, key).await? {
                tx.commit().await.map_err(|source| Error::Sql {
                    context: "结束接收事务",
                    source,
                })?;
                return Ok(existing);
            }
        }
        let mut heads = load_heads(&mut tx).await?;
        let resolved = resolve::resolve(changeset, &mut heads, commit_id.as_str(), &now_text)?;
        if let Some(task_id) = &resolved.ack {
            ack_task(&mut tx, task_id).await?;
        }
        insert_resolved(
            &mut tx,
            &resolved,
            changeset.idempotency_key.as_deref(),
            &now_text,
        )
        .await?;
        tx.commit().await.map_err(|source| Error::Sql {
            context: "提交接收事务",
            source,
        })?;
        Ok(CommitResult {
            commit_id: commit_id.clone(),
            status: WaitFor::Accepted,
            snapshot: None,
            duplicate: false,
        })
    }

    async fn list_accepted(&self) -> Result<Vec<resolve::StoredCommit>, Error> {
        let rows = sqlx::query("SELECT payload FROM outbox WHERE status = 'accepted' ORDER BY id")
            .fetch_all(&self.pool)
            .await
            .map_err(|source| Error::Sql {
                context: "读取 outbox",
                source,
            })?;
        rows.into_iter()
            .map(|row| {
                let payload: String = row.get(0);
                serde_json::from_str(&payload).map_err(Error::from)
            })
            .collect()
    }

    async fn flush_batch(&self) -> Result<Option<FlushBatch>, Error> {
        let row = sqlx::query("SELECT value FROM meta WHERE key = 'flush_batch'")
            .fetch_optional(&self.pool)
            .await
            .map_err(|source| Error::Sql {
                context: "读取刷写批次",
                source,
            })?;
        row.map(|row| {
            let value: String = row.get(0);
            serde_json::from_str(&value).map_err(Error::from)
        })
        .transpose()
    }

    async fn put_flush_batch(&self, batch: &FlushBatch) -> Result<(), Error> {
        let value = serde_json::to_string(batch)?;
        sqlx::query(
            "INSERT INTO meta(key, value) VALUES ('flush_batch', ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(value)
        .execute(&self.pool)
        .await
        .map_err(|source| Error::Sql {
            context: "写入刷写批次",
            source,
        })?;
        Ok(())
    }

    async fn finish_visible(&self, commit_ids: &[String], snapshot: u64) -> Result<(), Error> {
        let mut tx = self.pool.begin().await.map_err(|source| Error::Sql {
            context: "开始可见事务",
            source,
        })?;
        for commit_id in commit_ids {
            sqlx::query(
                "UPDATE outbox SET status = 'visible', snapshot_seq = ? WHERE commit_id = ?",
            )
            .bind(i64::try_from(snapshot).map_err(|_| Error::ColumnType(snapshot.to_string()))?)
            .bind(commit_id)
            .execute(&mut *tx)
            .await
            .map_err(|source| Error::Sql {
                context: "标记可见",
                source,
            })?;
        }
        for commit_id in commit_ids {
            sqlx::query("DELETE FROM staging_ref WHERE commit_id = ?")
                .bind(commit_id)
                .execute(&mut *tx)
                .await
                .map_err(|source| Error::Sql {
                    context: "删除暂存引用",
                    source,
                })?;
        }
        sqlx::query("DELETE FROM meta WHERE key = 'flush_batch'")
            .execute(&mut *tx)
            .await
            .map_err(|source| Error::Sql {
                context: "删除刷写批次",
                source,
            })?;
        tx.commit().await.map_err(|source| Error::Sql {
            context: "提交可见事务",
            source,
        })?;
        Ok(())
    }

    async fn clear_flush_batch(&self) -> Result<(), Error> {
        sqlx::query("DELETE FROM meta WHERE key = 'flush_batch'")
            .execute(&self.pool)
            .await
            .map_err(|source| Error::Sql {
                context: "删除刷写批次",
                source,
            })?;
        Ok(())
    }

    async fn baseline(&self, table: &str) -> Result<Option<u64>, Error> {
        read_counter(&self.pool, &format!("baseline:{table}")).await
    }

    async fn set_baseline(&self, table: &str, version: u64) -> Result<(), Error> {
        write_counter(&self.pool, &format!("baseline:{table}"), version).await
    }

    async fn alloc_seq(&self, count: u64) -> Result<u64, Error> {
        alloc_counter(&self.pool, "next_seq", count).await
    }

    async fn alloc_snapshot(&self) -> Result<u64, Error> {
        alloc_counter(&self.pool, "next_snapshot", 1).await
    }

    async fn enqueue(&self, tasks: &[AccessTask], now: Timestamp) -> Result<(), Error> {
        let now_text = now.to_rfc3339()?;
        let mut tx = self.pool.begin().await.map_err(|source| Error::Sql {
            context: "开始入队事务",
            source,
        })?;
        for task in tasks {
            task.validate()?;
            let exists = sqlx::query("SELECT 1 FROM tasks WHERE task_id = ?")
                .bind(task.task_id.as_str())
                .fetch_optional(&mut *tx)
                .await
                .map_err(|source| Error::Sql {
                    context: "查找任务",
                    source,
                })?;
            if exists.is_some() {
                return Err(Error::DuplicateTask(task.task_id.to_string()));
            }
            let locator_key = serde_json::to_string(&task.locator)?;
            let active = sqlx::query(
                "SELECT 1 FROM tasks WHERE source_id = ? AND locator_key = ? AND status != 'done' LIMIT 1",
            )
            .bind(task.source_id.as_str())
            .bind(&locator_key)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|source| Error::Sql {
                context: "查找在途任务",
                source,
            })?;
            if active.is_some() {
                continue;
            }
            sqlx::query(
                "INSERT INTO tasks(task_id, queue, payload, status, priority, created_at, source_id, locator_key)
                 VALUES (?, ?, ?, 'queued', ?, ?, ?, ?)",
            )
            .bind(task.task_id.as_str())
            .bind(queue_text(task.queue))
            .bind(serde_json::to_string(task)?)
            .bind(task.priority)
            .bind(&now_text)
            .bind(task.source_id.as_str())
            .bind(&locator_key)
            .execute(&mut *tx)
            .await
            .map_err(|source| Error::Sql {
                context: "写入任务",
                source,
            })?;
        }
        tx.commit().await.map_err(|source| Error::Sql {
            context: "提交入队事务",
            source,
        })?;
        Ok(())
    }

    async fn lease(
        &self,
        queue: QueueName,
        count: usize,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<Vec<AccessTask>, Error> {
        if count == 0 {
            return Ok(Vec::new());
        }
        let until = now.checked_add(lease_for)?;
        let until_text = until.to_rfc3339()?;
        let mut tx = self.pool.begin().await.map_err(|source| Error::Sql {
            context: "开始租约事务",
            source,
        })?;
        let rows = sqlx::query(
            "SELECT task_id, payload, status, lease_until, retry_at, priority, created_at
             FROM tasks WHERE queue = ? AND status IN ('queued', 'leased')",
        )
        .bind(queue_text(queue))
        .fetch_all(&mut *tx)
        .await
        .map_err(|source| Error::Sql {
            context: "读取队列",
            source,
        })?;
        let mut candidates = Vec::new();
        for row in rows {
            let status: String = row.get(2);
            let lease_until: Option<String> = row.get(3);
            let retry_at: Option<String> = row.get(4);
            let created_at: String = row.get(6);
            if !lease_due(&status, lease_until.as_deref(), now)? {
                continue;
            }
            if let Some(retry_at) = retry_at.as_deref() {
                if Timestamp::parse(retry_at)? > now {
                    continue;
                }
            }
            candidates.push(Candidate {
                task_id: row.get(0),
                payload: row.get(1),
                priority: row.get(5),
                created_at: Timestamp::parse(&created_at)?,
            });
        }
        candidates.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| left.created_at.cmp(&right.created_at))
        });
        candidates.truncate(count);
        let mut leased = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            let mut task: AccessTask = serde_json::from_str(&candidate.payload)?;
            task.lease_until = until;
            let payload = serde_json::to_string(&task)?;
            sqlx::query(
                "UPDATE tasks SET payload = ?, status = 'leased', lease_until = ? WHERE task_id = ?",
            )
            .bind(&payload)
            .bind(&until_text)
            .bind(&candidate.task_id)
            .execute(&mut *tx)
            .await
            .map_err(|source| Error::Sql { context: "更新租约", source })?;
            leased.push(task);
        }
        tx.commit().await.map_err(|source| Error::Sql {
            context: "提交租约事务",
            source,
        })?;
        Ok(leased)
    }

    async fn extend(
        &self,
        task_id: &TaskId,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<(), Error> {
        let until = now.checked_add(lease_for)?;
        let until_text = until.to_rfc3339()?;
        let row = sqlx::query("SELECT payload, status FROM tasks WHERE task_id = ?")
            .bind(task_id.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(|source| Error::Sql {
                context: "读取任务",
                source,
            })?;
        let Some(row) = row else {
            return Err(Error::UnknownTask(task_id.to_string()));
        };
        let status: String = row.get(1);
        if status != "leased" {
            return Err(Error::TaskState(task_id.to_string()));
        }
        let mut task: AccessTask = serde_json::from_str(row.get::<String, _>(0).as_str())?;
        task.lease_until = until;
        sqlx::query("UPDATE tasks SET payload = ?, lease_until = ? WHERE task_id = ?")
            .bind(serde_json::to_string(&task)?)
            .bind(until_text)
            .bind(task_id.as_str())
            .execute(&self.pool)
            .await
            .map_err(|source| Error::Sql {
                context: "延长租约",
                source,
            })?;
        Ok(())
    }

    async fn nack(&self, task_id: &TaskId, retry_at: Timestamp, error: &str) -> Result<(), Error> {
        let row = sqlx::query("SELECT payload, status FROM tasks WHERE task_id = ?")
            .bind(task_id.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(|source| Error::Sql {
                context: "读取任务",
                source,
            })?;
        let Some(row) = row else {
            return Err(Error::UnknownTask(task_id.to_string()));
        };
        let status: String = row.get(1);
        if status == "done" {
            return Err(Error::TaskState(task_id.to_string()));
        }
        let mut task: AccessTask = serde_json::from_str(row.get::<String, _>(0).as_str())?;
        task.attempt = task.attempt.saturating_add(1);
        sqlx::query(
            "UPDATE tasks SET payload = ?, status = 'queued', lease_until = NULL, retry_at = ?, last_error = ?
             WHERE task_id = ?",
        )
        .bind(serde_json::to_string(&task)?)
        .bind(retry_at.to_rfc3339()?)
        .bind(error)
        .bind(task_id.as_str())
        .execute(&self.pool)
        .await
        .map_err(|source| Error::Sql { context: "退回任务", source })?;
        Ok(())
    }

    async fn lease_id(
        &self,
        task_id: &TaskId,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<AccessTask, Error> {
        let until = now.checked_add(lease_for)?;
        let until_text = until.to_rfc3339()?;
        let row = sqlx::query(
            "SELECT payload, status, lease_until, retry_at FROM tasks WHERE task_id = ?",
        )
        .bind(task_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|source| Error::Sql {
            context: "读取任务",
            source,
        })?;
        let Some(row) = row else {
            return Err(Error::UnknownTask(task_id.to_string()));
        };
        let status: String = row.get(1);
        let lease_until: Option<String> = row.get(2);
        let retry_at: Option<String> = row.get(3);
        if status == "done" || !lease_due(&status, lease_until.as_deref(), now)? {
            return Err(Error::TaskState(task_id.to_string()));
        }
        if let Some(retry_at) = retry_at.as_deref() {
            if Timestamp::parse(retry_at)? > now {
                return Err(Error::TaskState(task_id.to_string()));
            }
        }
        let mut task: AccessTask = serde_json::from_str(row.get::<String, _>(0).as_str())?;
        task.lease_until = until;
        sqlx::query(
            "UPDATE tasks SET payload = ?, status = 'leased', lease_until = ? WHERE task_id = ?",
        )
        .bind(serde_json::to_string(&task)?)
        .bind(&until_text)
        .bind(task_id.as_str())
        .execute(&self.pool)
        .await
        .map_err(|source| Error::Sql {
            context: "更新租约",
            source,
        })?;
        Ok(task)
    }

    async fn release_leases(&self) -> Result<Vec<AccessTask>, Error> {
        let mut tx = self.pool.begin().await.map_err(|source| Error::Sql {
            context: "开始释放租约",
            source,
        })?;
        let rows = sqlx::query("SELECT task_id, payload FROM tasks WHERE status = 'leased'")
            .fetch_all(&mut *tx)
            .await
            .map_err(|source| Error::Sql {
                context: "读取租约",
                source,
            })?;
        let mut tasks = Vec::new();
        let released_at = Timestamp::now();
        for row in rows {
            let task_id: String = row.get(0);
            let mut task: AccessTask = serde_json::from_str(row.get::<String, _>(1).as_str())?;
            tasks.push(task.clone());
            task.lease_until = released_at;
            sqlx::query(
                "UPDATE tasks SET payload = ?, status = 'queued', lease_until = NULL WHERE task_id = ?",
            )
            .bind(serde_json::to_string(&task)?)
            .bind(task_id)
            .execute(&mut *tx)
            .await
            .map_err(|source| Error::Sql {
                context: "释放租约",
                source,
            })?;
        }
        tx.commit().await.map_err(|source| Error::Sql {
            context: "提交释放租约",
            source,
        })?;
        Ok(tasks)
    }

    async fn leased_count(&self, queue: QueueName, now: Timestamp) -> Result<usize, Error> {
        let rows =
            sqlx::query("SELECT lease_until FROM tasks WHERE queue = ? AND status = 'leased'")
                .bind(queue_text(queue))
                .fetch_all(&self.pool)
                .await
                .map_err(|source| Error::Sql {
                    context: "统计租约",
                    source,
                })?;
        let mut count = 0usize;
        for row in rows {
            let until: Option<String> = row.get(0);
            if let Some(until) = until {
                if Timestamp::parse(&until)? > now {
                    count += 1;
                }
            }
        }
        Ok(count)
    }

    async fn lease_ready(
        &self,
        source: &str,
        locator_key: &str,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<Option<AccessTask>, Error> {
        let row = sqlx::query(
            "SELECT task_id FROM tasks WHERE source_id = ? AND locator_key = ? AND status != 'done' LIMIT 1",
        )
        .bind(source)
        .bind(locator_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|source| Error::Sql {
            context: "查找可领任务",
            source,
        })?;
        let Some(row) = row else {
            return Ok(None);
        };
        let task_id = TaskId::new(row.get::<String, _>(0))?;
        match self.lease_id(&task_id, lease_for, now).await {
            Ok(task) => Ok(Some(task)),
            Err(Error::TaskState(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn task_active(&self, source: &str, locator_key: &str) -> Result<bool, Error> {
        let row = sqlx::query(
            "SELECT 1 FROM tasks WHERE source_id = ? AND locator_key = ? AND status != 'done' LIMIT 1",
        )
        .bind(source)
        .bind(locator_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|source| Error::Sql {
            context: "查找在途任务",
            source,
        })?;
        Ok(row.is_some())
    }

    async fn accepted_state(&self, scope: &str, key: &str) -> Result<Option<(u64, String)>, Error> {
        let row = sqlx::query("SELECT version, value FROM state_head WHERE scope = ? AND key = ?")
            .bind(scope)
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|source| Error::Sql {
                context: "读取已接收状态",
                source,
            })?;
        let Some(row) = row else {
            return Ok(None);
        };
        let version: i64 = row.get(0);
        Ok(Some((u64_from_i64(version)?, row.get(1))))
    }

    async fn hot_get(&self, kind: &str, key: &str) -> Result<Option<String>, Error> {
        let row = sqlx::query("SELECT value FROM hot WHERE kind = ? AND key = ?")
            .bind(kind)
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|source| Error::Sql {
                context: "读取热数据",
                source,
            })?;
        Ok(row.map(|row| row.get(0)))
    }

    async fn hot_put(&self, kind: &str, key: &str, value: &str) -> Result<(), Error> {
        sqlx::query(
            "INSERT INTO hot(kind, key, value) VALUES (?, ?, ?)
             ON CONFLICT(kind, key) DO UPDATE SET value = excluded.value",
        )
        .bind(kind)
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await
        .map_err(|source| Error::Sql {
            context: "写入热数据",
            source,
        })?;
        Ok(())
    }

    async fn staging_for(&self, commit_ids: &[String]) -> Result<Vec<String>, Error> {
        let mut shas = Vec::new();
        for commit_id in commit_ids {
            let rows = sqlx::query("SELECT sha256 FROM staging_ref WHERE commit_id = ?")
                .bind(commit_id)
                .fetch_all(&self.pool)
                .await
                .map_err(|source| Error::Sql {
                    context: "读取暂存引用",
                    source,
                })?;
            for row in rows {
                shas.push(row.get(0));
            }
        }
        Ok(shas)
    }

    async fn accepted_staging(&self) -> Result<HashSet<String>, Error> {
        let rows = sqlx::query("SELECT sha256 FROM staging_ref")
            .fetch_all(&self.pool)
            .await
            .map_err(|source| Error::Sql {
                context: "读取暂存引用",
                source,
            })?;
        Ok(rows.into_iter().map(|row| row.get(0)).collect())
    }

    async fn outcome(&self, commit_id: &str) -> Result<Option<CommitResult>, Error> {
        let row = sqlx::query("SELECT status, snapshot_seq FROM outbox WHERE commit_id = ?")
            .bind(commit_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|source| Error::Sql {
                context: "读取提交",
                source,
            })?;
        let Some(row) = row else {
            return Ok(None);
        };
        let status: String = row.get(0);
        let snapshot: Option<i64> = row.get(1);
        let snapshot = snapshot.map(u64_from_i64).transpose()?;
        Ok(Some(CommitResult {
            commit_id: CommitId::new(commit_id)?,
            status: if snapshot.is_some() && status == "visible" {
                WaitFor::Visible
            } else {
                WaitFor::Accepted
            },
            snapshot,
            duplicate: false,
        }))
    }
}

struct Candidate {
    task_id: String,
    payload: String,
    priority: i32,
    created_at: Timestamp,
}

async fn duplicate(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    key: &str,
) -> Result<Option<CommitResult>, Error> {
    let row =
        sqlx::query("SELECT commit_id, status, snapshot_seq FROM outbox WHERE idempotency_key = ?")
            .bind(key)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|source| Error::Sql {
                context: "查找幂等键",
                source,
            })?;
    let Some(row) = row else {
        return Ok(None);
    };
    let commit_id: String = row.get(0);
    let status: String = row.get(1);
    let snapshot: Option<i64> = row.get(2);
    let snapshot = snapshot.map(u64_from_i64).transpose()?;
    Ok(Some(CommitResult {
        commit_id: CommitId::new(commit_id)?,
        status: if snapshot.is_some() && status == "visible" {
            WaitFor::Visible
        } else {
            WaitFor::Accepted
        },
        snapshot,
        duplicate: true,
    }))
}

async fn load_heads(tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>) -> Result<HeadSet, Error> {
    let mut heads = HeadSet::default();
    let identities = sqlx::query("SELECT identity_key, rid, tombstone FROM identity_assign")
        .fetch_all(&mut **tx)
        .await
        .map_err(|source| Error::Sql {
            context: "读取身份",
            source,
        })?;
    for row in identities {
        let tombstone: i64 = row.get(2);
        heads.identities.insert(
            row.get(0),
            resolve::IdentityHead {
                rid: row.get(1),
                tombstone: tombstone != 0,
            },
        );
    }
    let resources = sqlx::query("SELECT rid, tombstone FROM resource_head")
        .fetch_all(&mut **tx)
        .await
        .map_err(|source| Error::Sql {
            context: "读取资源",
            source,
        })?;
    for row in resources {
        let tombstone: i64 = row.get(1);
        heads.resources.insert(
            row.get(0),
            resolve::ResourceHead {
                tombstone: tombstone != 0,
            },
        );
    }
    let states = sqlx::query("SELECT scope, key, version FROM state_head")
        .fetch_all(&mut **tx)
        .await
        .map_err(|source| Error::Sql {
            context: "读取状态",
            source,
        })?;
    for row in states {
        let version: i64 = row.get(2);
        heads
            .states
            .insert((row.get(0), row.get(1)), u64_from_i64(version)?);
    }
    Ok(heads)
}

async fn insert_resolved(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    resolved: &Resolved,
    idempotency_key: Option<&str>,
    now: &str,
) -> Result<(), Error> {
    let payload = serde_json::to_string(&resolved.commit)?;
    sqlx::query(
        "INSERT INTO outbox(commit_id, idempotency_key, payload, status, created_at)
         VALUES (?, ?, ?, 'accepted', ?)",
    )
    .bind(&resolved.commit.commit_id)
    .bind(idempotency_key)
    .bind(payload)
    .bind(now)
    .execute(&mut **tx)
    .await
    .map_err(|source| Error::Sql {
        context: "写入 outbox",
        source,
    })?;
    for identity in &resolved.identities {
        sqlx::query(
            "INSERT INTO identity_assign(identity_key, rid, scheme, source_id, tombstone, commit_id)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&identity.key)
        .bind(&identity.rid)
        .bind(&identity.scheme)
        .bind(&identity.source_id)
        .bind(i64::from(u8::from(identity.tombstone)))
        .bind(&resolved.commit.commit_id)
        .execute(&mut **tx)
        .await
        .map_err(|source| Error::Sql { context: "写入身份", source })?;
    }
    for resource in &resolved.resources {
        sqlx::query(
            "INSERT INTO resource_head(rid, ns, source_id, created_at, tombstone, commit_id)
             VALUES (?, ?, ?, ?, 0, ?)",
        )
        .bind(&resource.rid)
        .bind(&resource.ns)
        .bind(&resource.source_id)
        .bind(&resource.created_at)
        .bind(&resolved.commit.commit_id)
        .execute(&mut **tx)
        .await
        .map_err(|source| Error::Sql {
            context: "写入资源",
            source,
        })?;
    }
    for rid in &resolved.tombstones {
        sqlx::query("UPDATE identity_assign SET tombstone = 1 WHERE rid = ?")
            .bind(rid)
            .execute(&mut **tx)
            .await
            .map_err(|source| Error::Sql {
                context: "写入墓碑",
                source,
            })?;
        sqlx::query("UPDATE resource_head SET tombstone = 1 WHERE rid = ?")
            .bind(rid)
            .execute(&mut **tx)
            .await
            .map_err(|source| Error::Sql {
                context: "写入墓碑",
                source,
            })?;
    }
    for state in &resolved.states {
        sqlx::query(
            "INSERT INTO state_head(scope, key, version, value, commit_id) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(scope, key) DO UPDATE SET version = excluded.version, value = excluded.value, commit_id = excluded.commit_id",
        )
        .bind(&state.scope)
        .bind(&state.key)
        .bind(i64::try_from(state.version).map_err(|_| Error::ColumnType(state.version.to_string()))?)
        .bind(&state.value)
        .bind(&resolved.commit.commit_id)
        .execute(&mut **tx)
        .await
        .map_err(|source| Error::Sql { context: "写入状态", source })?;
    }
    for sha in &resolved.staging {
        sqlx::query("INSERT INTO staging_ref(sha256, commit_id) VALUES (?, ?)")
            .bind(sha)
            .bind(&resolved.commit.commit_id)
            .execute(&mut **tx)
            .await
            .map_err(|source| Error::Sql {
                context: "写入暂存引用",
                source,
            })?;
    }
    Ok(())
}

async fn ack_task(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    task_id: &str,
) -> Result<(), Error> {
    let row = sqlx::query("SELECT status FROM tasks WHERE task_id = ?")
        .bind(task_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|source| Error::Sql {
            context: "读取任务",
            source,
        })?;
    let Some(row) = row else {
        return Err(Error::UnknownTask(task_id.to_string()));
    };
    let status: String = row.get(0);
    if status == "done" {
        return Err(Error::TaskState(task_id.to_string()));
    }
    sqlx::query("UPDATE tasks SET status = 'done', lease_until = NULL WHERE task_id = ?")
        .bind(task_id)
        .execute(&mut **tx)
        .await
        .map_err(|source| Error::Sql {
            context: "确认任务",
            source,
        })?;
    Ok(())
}

async fn ensure_task_column(pool: &SqlitePool, name: &str, decl: &str) -> Result<(), Error> {
    let rows = sqlx::query("PRAGMA table_info(tasks)")
        .fetch_all(pool)
        .await
        .map_err(|source| Error::Sql {
            context: "读取任务表结构",
            source,
        })?;
    let exists = rows.iter().any(|row| row.get::<String, _>(1) == name);
    if exists {
        return Ok(());
    }
    let sql = format!("ALTER TABLE tasks ADD COLUMN {name} {decl}");
    sqlx::query(&sql)
        .execute(pool)
        .await
        .map_err(|source| Error::Sql {
            context: "补充任务列",
            source,
        })?;
    Ok(())
}

fn lease_due(status: &str, lease_until: Option<&str>, now: Timestamp) -> Result<bool, Error> {
    match status {
        "queued" => Ok(true),
        "leased" => {
            let Some(lease_until) = lease_until else {
                return Ok(true);
            };
            Ok(Timestamp::parse(lease_until)? <= now)
        }
        _ => Ok(false),
    }
}

fn queue_text(queue: QueueName) -> &'static str {
    match queue {
        QueueName::Free => "free",
        QueueName::Paid => "paid",
        QueueName::Render => "render",
        QueueName::Push => "push",
    }
}

fn u64_from_i64(value: i64) -> Result<u64, Error> {
    u64::try_from(value).map_err(|_| Error::ColumnType(value.to_string()))
}

async fn read_counter(pool: &SqlitePool, key: &str) -> Result<Option<u64>, Error> {
    let row = sqlx::query("SELECT value FROM meta WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .map_err(|source| Error::Sql {
            context: "读取计数",
            source,
        })?;
    row.map(|row| {
        let text: String = row.get(0);
        text.parse::<u64>().map_err(|_| Error::ColumnType(text))
    })
    .transpose()
}

async fn write_counter(pool: &SqlitePool, key: &str, value: u64) -> Result<(), Error> {
    sqlx::query(
        "INSERT INTO meta(key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value.to_string())
    .execute(pool)
    .await
    .map_err(|source| Error::Sql {
        context: "写入计数",
        source,
    })?;
    Ok(())
}

async fn alloc_counter(pool: &SqlitePool, key: &str, count: u64) -> Result<u64, Error> {
    let mut tx = pool.begin().await.map_err(|source| Error::Sql {
        context: "开始计数事务",
        source,
    })?;
    let current = read_counter_tx(&mut tx, key).await?.unwrap_or(1);
    if count > 0 {
        let next = current
            .checked_add(count)
            .ok_or_else(|| Error::ColumnType(key.to_string()))?;
        sqlx::query(
            "INSERT INTO meta(key, value) VALUES (?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(next.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|source| Error::Sql {
            context: "写入计数",
            source,
        })?;
    }
    tx.commit().await.map_err(|source| Error::Sql {
        context: "提交计数事务",
        source,
    })?;
    Ok(current)
}

async fn read_counter_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    key: &str,
) -> Result<Option<u64>, Error> {
    let row = sqlx::query("SELECT value FROM meta WHERE key = ?")
        .bind(key)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|source| Error::Sql {
            context: "读取计数",
            source,
        })?;
    row.map(|row| {
        let text: String = row.get(0);
        text.parse::<u64>().map_err(|_| Error::ColumnType(text))
    })
    .transpose()
}
