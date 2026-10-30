//! 控制面存储：Operation、领域文档、观测流与维护预演。
//!
//! 与 Strata 元数据库分离：Strata 管内容，这里管控制面的业务状态。
//! 所有表都在同一个 SQLite 文件内，写事务使用 `BEGIN IMMEDIATE` 避免并发写冲突。

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Row, SqlitePool};

use crate::error::ApiError;

/// 数据库文件名。放在数据目录下，与控制面一起迁移。
const DB_FILE: &str = "control.db";

#[derive(Clone, Copy, Debug, Default)]
pub struct RecordFilter<'a> {
    pub level: Option<&'a str>,
    pub kind: Option<&'a str>,
    pub correlation: Option<&'a str>,
    pub query: Option<&'a str>,
}

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

#[derive(Clone, Debug)]
pub struct Document {
    pub id: String,
    pub body: Value,
    pub version: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug)]
pub struct StoredOperation {
    pub operation: Value,
    pub action: Value,
    pub request_id: String,
}

impl Store {
    pub async fn open(data_dir: &Path) -> Result<Self, ApiError> {
        tokio::fs::create_dir_all(data_dir)
            .await
            .map_err(|error| ApiError::internal(format!("创建数据目录失败: {error}")))?;
        let path = data_dir.join(DB_FILE);
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(10));
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options)
            .await
            .map_err(|error| ApiError::internal(format!("打开控制面数据库失败: {error}")))?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    async fn migrate(&self) -> Result<(), ApiError> {
        let statements = [
            "CREATE TABLE IF NOT EXISTS documents (
                kind TEXT NOT NULL,
                id TEXT NOT NULL,
                body TEXT NOT NULL,
                version INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (kind, id)
            )",
            "CREATE INDEX IF NOT EXISTS documents_kind ON documents(kind, created_at DESC)",
            "CREATE TABLE IF NOT EXISTS operations (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                status TEXT NOT NULL,
                progress REAL NOT NULL,
                cancellable INTEGER NOT NULL,
                retryable INTEGER NOT NULL,
                retry_of TEXT,
                resource_refs TEXT NOT NULL,
                result TEXT,
                error TEXT,
                action TEXT NOT NULL,
                request_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                completed_at TEXT
            )",
            "CREATE INDEX IF NOT EXISTS operations_created ON operations(created_at DESC)",
            "CREATE TABLE IF NOT EXISTS operation_events (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                operation_id TEXT NOT NULL,
                at TEXT NOT NULL,
                kind TEXT NOT NULL,
                message TEXT NOT NULL,
                data TEXT
            )",
            "CREATE INDEX IF NOT EXISTS operation_events_op ON operation_events(operation_id, seq)",
            "CREATE TABLE IF NOT EXISTS records (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                stream TEXT NOT NULL,
                at TEXT NOT NULL,
                level TEXT NOT NULL,
                kind TEXT NOT NULL,
                message TEXT NOT NULL,
                data TEXT,
                request_id TEXT,
                operation_id TEXT,
                task_id TEXT,
                source_id TEXT,
                content_id TEXT,
                run_id TEXT,
                attempt_id TEXT,
                delivery_id TEXT,
                config_generation INTEGER
            )",
            "CREATE INDEX IF NOT EXISTS records_stream ON records(stream, seq DESC)",
            "CREATE INDEX IF NOT EXISTS records_filter ON records(stream, level, kind, at DESC)",
            "CREATE TABLE IF NOT EXISTS idempotency (
                key TEXT PRIMARY KEY,
                fingerprint TEXT NOT NULL,
                status INTEGER NOT NULL,
                body TEXT NOT NULL,
                created_at TEXT NOT NULL
            )",
            "CREATE TABLE IF NOT EXISTS previews (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                body TEXT NOT NULL,
                token TEXT NOT NULL,
                etag TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                created_at TEXT NOT NULL
            )",
        ];
        for statement in statements {
            sqlx::query(statement)
                .execute(&self.pool)
                .await
                .map_err(|error| ApiError::internal(format!("初始化控制面表失败: {error}")))?;
        }
        Ok(())
    }

    pub async fn get_document(&self, kind: &str, id: &str) -> Result<Option<Document>, ApiError> {
        let row = sqlx::query(
            "SELECT id, body, version, created_at, updated_at FROM documents WHERE kind = ? AND id = ?",
        )
        .bind(kind)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        Ok(row.map(document))
    }

    pub async fn list_documents(&self, kind: &str) -> Result<Vec<Document>, ApiError> {
        let rows = sqlx::query(
            "SELECT id, body, version, created_at, updated_at FROM documents WHERE kind = ? ORDER BY created_at DESC, id DESC",
        )
        .bind(kind)
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        Ok(rows.into_iter().map(document).collect())
    }

    /// 新建文档：同 id 已存在时返回冲突，避免静默覆盖。
    pub async fn insert_document(
        &self,
        kind: &str,
        id: &str,
        body: &Value,
    ) -> Result<Document, ApiError> {
        let now = crate::now();
        let text = serde_json::to_string(body)
            .map_err(|error| ApiError::internal(format!("序列化文档失败: {error}")))?;
        sqlx::query(
            "INSERT INTO documents(kind, id, body, version, created_at, updated_at) VALUES (?, ?, ?, 1, ?, ?)",
        )
        .bind(kind)
        .bind(id)
        .bind(text)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|error| {
            if is_unique_violation(&error) {
                ApiError::conflict(format!("资源已存在: {id}"))
            } else {
                db(error)
            }
        })?;
        self.get_document(kind, id)
            .await?
            .ok_or_else(|| ApiError::internal("写入后读取文档失败"))
    }

    /// 更新文档。`expected` 为读取到的版本；不匹配返回 412。
    pub async fn update_document(
        &self,
        kind: &str,
        id: &str,
        body: &Value,
        expected: Option<i64>,
    ) -> Result<Document, ApiError> {
        let now = crate::now();
        let text = serde_json::to_string(body)
            .map_err(|error| ApiError::internal(format!("序列化文档失败: {error}")))?;
        let result = match expected {
            Some(version) => {
                sqlx::query(
                    "UPDATE documents SET body = ?, version = version + 1, updated_at = ? WHERE kind = ? AND id = ? AND version = ?",
                )
                .bind(text)
                .bind(&now)
                .bind(kind)
                .bind(id)
                .bind(version)
                .execute(&self.pool)
                .await
                .map_err(db)?
            }
            None => {
                sqlx::query(
                    "UPDATE documents SET body = ?, version = version + 1, updated_at = ? WHERE kind = ? AND id = ?",
                )
                .bind(text)
                .bind(&now)
                .bind(kind)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(db)?
            }
        };
        if result.rows_affected() == 0 {
            return match self.get_document(kind, id).await? {
                Some(_) => Err(ApiError::precondition("资源已被其他会话修改")),
                None => Err(ApiError::not_found(id)),
            };
        }
        self.get_document(kind, id)
            .await?
            .ok_or_else(|| ApiError::internal("更新后读取文档失败"))
    }

    pub async fn delete_document(&self, kind: &str, id: &str) -> Result<bool, ApiError> {
        let result = sqlx::query("DELETE FROM documents WHERE kind = ? AND id = ?")
            .bind(kind)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_kind(&self, kind: &str) -> Result<u64, ApiError> {
        let result = sqlx::query("DELETE FROM documents WHERE kind = ?")
            .bind(kind)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(result.rows_affected())
    }

    pub async fn count_kind(&self, kind: &str) -> Result<u64, ApiError> {
        let row = sqlx::query("SELECT COUNT(*) AS n FROM documents WHERE kind = ?")
            .bind(kind)
            .fetch_one(&self.pool)
            .await
            .map_err(db)?;
        Ok(row.try_get::<i64, _>("n").unwrap_or(0).max(0) as u64)
    }

    pub async fn insert_operation(
        &self,
        operation: &Value,
        action: &Value,
        request_id: &str,
    ) -> Result<(), ApiError> {
        let now = crate::now();
        sqlx::query(
            "INSERT INTO operations(id, kind, status, progress, cancellable, retryable, retry_of, resource_refs, result, error, action, request_id, created_at, updated_at, completed_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(text(operation, "operationId"))
        .bind(text(operation, "kind"))
        .bind(text(operation, "status"))
        .bind(number(operation, "progress"))
        .bind(flag(operation, "cancellable"))
        .bind(flag(operation, "retryable"))
        .bind(optional_text(operation, "retryOf"))
        .bind(
            operation
                .get("resourceRefs")
                .map(Value::to_string)
                .unwrap_or_else(|| "[]".into()),
        )
        .bind(optional_text(operation, "resultSummary"))
        .bind(optional_text(operation, "errorSummary"))
        .bind(action.to_string())
        .bind(request_id)
        .bind(&now)
        .bind(&now)
        .bind(optional_text(operation, "completedAt"))
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    pub async fn get_operation(&self, id: &str) -> Result<Option<StoredOperation>, ApiError> {
        let row = sqlx::query("SELECT * FROM operations WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        Ok(row.map(operation_row))
    }

    pub async fn list_operations(
        &self,
        status: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Value>, ApiError> {
        let rows = sqlx::query(
            "SELECT * FROM operations WHERE (? IS NULL OR status = ?) ORDER BY created_at DESC, id DESC LIMIT ?",
        )
        .bind(status)
        .bind(status)
        .bind(limit as i64 + 1)
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        Ok(rows
            .into_iter()
            .map(|row| operation_row(row).operation)
            .collect())
    }

    pub async fn count_finished_operations(&self) -> Result<u64, ApiError> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS n FROM operations WHERE status IN ('succeeded', 'partiallySucceeded', 'failed', 'cancelled')",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(db)?;
        Ok(row.try_get::<i64, _>("n").unwrap_or_default().max(0) as u64)
    }

    pub async fn update_operation(&self, id: &str, patch: &Value) -> Result<(), ApiError> {
        let now = crate::now();
        let current = self
            .get_operation(id)
            .await?
            .ok_or_else(|| ApiError::not_found(id))?;
        let mut operation = current.operation;
        if let (Value::Object(target), Value::Object(patch)) = (&mut operation, patch) {
            for (key, value) in patch {
                target.insert(key.clone(), value.clone());
            }
        }
        sqlx::query(
            "UPDATE operations SET status = ?, progress = ?, cancellable = ?, retryable = ?, result = ?, error = ?, updated_at = ?, completed_at = ? WHERE id = ?",
        )
        .bind(text(&operation, "status"))
        .bind(number(&operation, "progress"))
        .bind(flag(&operation, "cancellable"))
        .bind(flag(&operation, "retryable"))
        .bind(optional_text(&operation, "resultSummary"))
        .bind(optional_text(&operation, "errorSummary"))
        .bind(&now)
        .bind(optional_text(&operation, "completedAt"))
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    pub async fn append_operation_event(
        &self,
        operation_id: &str,
        kind: &str,
        message: &str,
        data: Option<&Value>,
    ) -> Result<i64, ApiError> {
        let result = sqlx::query(
            "INSERT INTO operation_events(operation_id, at, kind, message, data) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(operation_id)
        .bind(crate::now())
        .bind(kind)
        .bind(message)
        .bind(data.map(Value::to_string))
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(result.last_insert_rowid())
    }

    pub async fn list_operation_events(
        &self,
        operation_id: &str,
        after: i64,
        limit: usize,
    ) -> Result<Vec<Value>, ApiError> {
        let rows = sqlx::query(
            "SELECT seq, at, kind, message, data FROM operation_events WHERE operation_id = ? AND seq > ? ORDER BY seq ASC LIMIT ?",
        )
        .bind(operation_id)
        .bind(after)
        .bind(limit as i64 + 1)
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        Ok(rows
            .into_iter()
            .map(|row| {
                json!({
                    "cursor": row.try_get::<i64, _>("seq").unwrap_or(0).to_string(),
                    "operationId": operation_id,
                    "timestamp": row.try_get::<String, _>("at").unwrap_or_default(),
                    "kind": row.try_get::<String, _>("kind").unwrap_or_default(),
                    "message": row.try_get::<String, _>("message").unwrap_or_default(),
                    "data": row.try_get::<Option<String>, _>("data").ok().flatten().and_then(|text| serde_json::from_str::<Value>(&text).ok()),
                })
            })
            .collect())
    }

    pub async fn append_record(&self, record: &Value, stream: &str) -> Result<i64, ApiError> {
        let result = sqlx::query(
            "INSERT INTO records(stream, at, level, kind, message, data, request_id, operation_id, task_id, source_id, content_id, run_id, attempt_id, delivery_id, config_generation)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(stream)
        .bind(text(record, "timestamp"))
        .bind(text(record, "level"))
        .bind(text(record, "kind"))
        .bind(text(record, "message"))
        .bind(record.get("data").map(Value::to_string))
        .bind(optional_text(record, "requestId"))
        .bind(optional_text(record, "operationId"))
        .bind(optional_text(record, "taskId"))
        .bind(optional_text(record, "sourceId"))
        .bind(optional_text(record, "contentId"))
        .bind(optional_text(record, "runId"))
        .bind(optional_text(record, "attemptId"))
        .bind(optional_text(record, "deliveryId"))
        .bind(record.get("configGeneration").and_then(Value::as_i64))
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(result.last_insert_rowid())
    }

    pub async fn list_records(
        &self,
        stream: &str,
        limit: usize,
        filter: RecordFilter<'_>,
    ) -> Result<Vec<(i64, Value)>, ApiError> {
        let RecordFilter {
            level,
            kind,
            correlation,
            query,
        } = filter;
        let needle = query.map(|value| format!("%{value}%"));
        let rows = sqlx::query(
            "SELECT seq, at, level, kind, message, data, request_id, operation_id, task_id, source_id, content_id, run_id, attempt_id, delivery_id, config_generation
             FROM records
             WHERE stream = ?
               AND (? IS NULL OR level = ?)
               AND (? IS NULL OR kind = ?)
               AND (? IS NULL OR request_id = ? OR operation_id = ? OR task_id = ? OR run_id = ? OR content_id = ? OR delivery_id = ?)
               AND (? IS NULL OR message LIKE ?)
             ORDER BY seq DESC LIMIT ?",
        )
        .bind(stream)
        .bind(level)
        .bind(level)
        .bind(kind)
        .bind(kind)
        .bind(correlation)
        .bind(correlation)
        .bind(correlation)
        .bind(correlation)
        .bind(correlation)
        .bind(correlation)
        .bind(correlation)
        .bind(&needle)
        .bind(&needle)
        .bind(limit as i64 + 1)
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let seq = row.try_get::<i64, _>("seq").unwrap_or(0);
                let value = json!({
                    "timestamp": row.try_get::<String, _>("at").unwrap_or_default(),
                    "level": row.try_get::<String, _>("level").unwrap_or_default(),
                    "kind": row.try_get::<String, _>("kind").unwrap_or_default(),
                    "message": row.try_get::<String, _>("message").unwrap_or_default(),
                    "requestId": row.try_get::<Option<String>, _>("request_id").ok().flatten(),
                    "operationId": row.try_get::<Option<String>, _>("operation_id").ok().flatten(),
                    "taskId": row.try_get::<Option<String>, _>("task_id").ok().flatten(),
                    "sourceId": row.try_get::<Option<String>, _>("source_id").ok().flatten(),
                    "contentId": row.try_get::<Option<String>, _>("content_id").ok().flatten(),
                    "runId": row.try_get::<Option<String>, _>("run_id").ok().flatten(),
                    "attemptId": row.try_get::<Option<String>, _>("attempt_id").ok().flatten(),
                    "deliveryId": row.try_get::<Option<String>, _>("delivery_id").ok().flatten(),
                    "configGeneration": row.try_get::<Option<i64>, _>("config_generation").ok().flatten(),
                    "data": row.try_get::<Option<String>, _>("data").ok().flatten().and_then(|text| serde_json::from_str::<Value>(&text).ok()),
                });
                (seq, value)
            })
            .collect())
    }

    pub async fn clear_records(&self, stream: &str) -> Result<u64, ApiError> {
        let result = sqlx::query("DELETE FROM records WHERE stream = ?")
            .bind(stream)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(result.rows_affected())
    }

    pub async fn count_records(&self, stream: &str) -> Result<u64, ApiError> {
        let row = sqlx::query("SELECT COUNT(*) AS n FROM records WHERE stream = ?")
            .bind(stream)
            .fetch_one(&self.pool)
            .await
            .map_err(db)?;
        Ok(row.try_get::<i64, _>("n").unwrap_or(0).max(0) as u64)
    }

    /// 与 list_records 同筛选口径的总数：页码分页的 total 用它，避免整窗拉取。
    pub async fn count_records_filtered(
        &self,
        stream: &str,
        filter: RecordFilter<'_>,
    ) -> Result<usize, ApiError> {
        let RecordFilter {
            level,
            kind,
            correlation,
            query,
        } = filter;
        let needle = query.map(|value| format!("%{value}%"));
        let row = sqlx::query(
            "SELECT COUNT(*) AS n FROM records
             WHERE stream = ?
               AND (? IS NULL OR level = ?)
               AND (? IS NULL OR kind = ?)
               AND (? IS NULL OR request_id = ? OR operation_id = ? OR task_id = ? OR run_id = ? OR content_id = ? OR delivery_id = ?)
               AND (? IS NULL OR message LIKE ?)",
        )
        .bind(stream)
        .bind(level)
        .bind(level)
        .bind(kind)
        .bind(kind)
        .bind(correlation)
        .bind(correlation)
        .bind(correlation)
        .bind(correlation)
        .bind(correlation)
        .bind(correlation)
        .bind(correlation)
        .bind(&needle)
        .bind(&needle)
        .fetch_one(&self.pool)
        .await
        .map_err(db)?;
        Ok(row.try_get::<i64, _>("n").unwrap_or(0).max(0) as usize)
    }

    pub async fn prune_records_before(&self, stream: &str, before: &str) -> Result<u64, ApiError> {
        let result = sqlx::query("DELETE FROM records WHERE stream = ? AND at < ?")
            .bind(stream)
            .bind(before)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(result.rows_affected())
    }

    /// 只保留某类文档最新的 `keep` 条，供健康历史等有界集合使用。
    pub async fn trim_documents(&self, kind: &str, keep: usize) -> Result<u64, ApiError> {
        let result = sqlx::query(
            "DELETE FROM documents WHERE kind = ? AND id NOT IN (SELECT id FROM documents WHERE kind = ? ORDER BY created_at DESC LIMIT ?)",
        )
        .bind(kind)
        .bind(kind)
        .bind(keep as i64)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(result.rows_affected())
    }

    /// 恢复出厂：清文档、观测流、幂等记录与历史操作，仅保留当前操作供报告查询。
    pub async fn reset_documents_keep_operation(&self, keep: &str) -> Result<(), ApiError> {
        for statement in [
            "DELETE FROM documents",
            "DELETE FROM records",
            "DELETE FROM idempotency",
            "DELETE FROM previews",
        ] {
            sqlx::query(statement)
                .execute(&self.pool)
                .await
                .map_err(db)?;
        }
        sqlx::query("DELETE FROM operation_events WHERE operation_id <> ?")
            .bind(keep)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        sqlx::query("DELETE FROM operations WHERE id <> ?")
            .bind(keep)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }

    pub async fn clear_finished_operations(&self) -> Result<u64, ApiError> {
        let result = sqlx::query(
            "DELETE FROM operations WHERE status IN ('succeeded', 'partiallySucceeded', 'failed', 'cancelled')",
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(result.rows_affected())
    }

    pub async fn idempotent(
        &self,
        key: &str,
        fingerprint: &str,
    ) -> Result<Option<(u16, Value)>, ApiError> {
        let row = sqlx::query("SELECT fingerprint, status, body FROM idempotency WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        let Some(row) = row else { return Ok(None) };
        let stored: String = row.try_get("fingerprint").unwrap_or_default();
        if stored != fingerprint {
            return Err(ApiError::idempotency_conflict());
        }
        let status = row.try_get::<i64, _>("status").unwrap_or(200) as u16;
        let body = row.try_get::<String, _>("body").unwrap_or_default();
        let value = serde_json::from_str(&body).unwrap_or(Value::Null);
        Ok(Some((status, value)))
    }

    /// 先占位再执行，避免同一幂等键的并发请求同时穿透。
    pub async fn reserve_idempotent(&self, key: &str, fingerprint: &str) -> Result<bool, ApiError> {
        let result = sqlx::query(
            "INSERT INTO idempotency(key, fingerprint, status, body, created_at) VALUES (?, ?, 0, '', ?) ON CONFLICT(key) DO NOTHING",
        )
        .bind(key)
        .bind(fingerprint)
        .bind(crate::now())
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(result.rows_affected() == 1)
    }

    /// 执行失败时撤销占位，允许客户端用同一键重试。
    pub async fn forget_idempotent(&self, key: &str) -> Result<(), ApiError> {
        sqlx::query("DELETE FROM idempotency WHERE key = ? AND status = 0")
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }

    pub async fn remember_idempotent(
        &self,
        key: &str,
        fingerprint: &str,
        status: u16,
        body: &Value,
    ) -> Result<(), ApiError> {
        sqlx::query(
            "INSERT OR REPLACE INTO idempotency(key, fingerprint, status, body, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(key)
        .bind(fingerprint)
        .bind(status as i64)
        .bind(body.to_string())
        .bind(crate::now())
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    pub async fn insert_preview(
        &self,
        id: &str,
        kind: &str,
        body: &Value,
        token: &str,
        etag: &str,
        expires_at: &str,
    ) -> Result<(), ApiError> {
        sqlx::query(
            "INSERT INTO previews(id, kind, body, token, etag, expires_at, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(kind)
        .bind(body.to_string())
        .bind(token)
        .bind(etag)
        .bind(expires_at)
        .bind(crate::now())
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    pub async fn get_preview(&self, id: &str) -> Result<Option<Value>, ApiError> {
        let row = sqlx::query("SELECT body FROM previews WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        Ok(row.and_then(|row| {
            row.try_get::<String, _>("body")
                .ok()
                .and_then(|text| serde_json::from_str(&text).ok())
        }))
    }

    pub async fn delete_preview(&self, id: &str) -> Result<(), ApiError> {
        sqlx::query("DELETE FROM previews WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }
}

fn document(row: sqlx::sqlite::SqliteRow) -> Document {
    let body: String = row.try_get("body").unwrap_or_else(|_| "{}".into());
    Document {
        id: row.try_get("id").unwrap_or_default(),
        body: serde_json::from_str(&body).unwrap_or(Value::Null),
        version: row.try_get("version").unwrap_or(1),
        created_at: row.try_get("created_at").unwrap_or_default(),
        updated_at: row.try_get("updated_at").unwrap_or_default(),
    }
}

fn operation_row(row: sqlx::sqlite::SqliteRow) -> StoredOperation {
    let result: Option<String> = row.try_get("result").ok().flatten();
    let error: Option<String> = row.try_get("error").ok().flatten();
    let refs: String = row.try_get("resource_refs").unwrap_or_else(|_| "[]".into());
    let operation = json!({
        "operationId": row.try_get::<String, _>("id").unwrap_or_default(),
        "kind": row.try_get::<String, _>("kind").unwrap_or_default(),
        "status": row.try_get::<String, _>("status").unwrap_or_default(),
        "progress": row.try_get::<f64, _>("progress").unwrap_or(0.0),
        "cancellable": row.try_get::<i64, _>("cancellable").unwrap_or(0) != 0,
        "retryable": row.try_get::<i64, _>("retryable").unwrap_or(0) != 0,
        "retryOf": row.try_get::<Option<String>, _>("retry_of").ok().flatten(),
        "resourceRefs": serde_json::from_str::<Value>(&refs).unwrap_or_else(|_| json!([])),
        "resultSummary": result.and_then(|text| serde_json::from_str::<Value>(&text).ok()),
        "errorSummary": error.and_then(|text| serde_json::from_str::<Value>(&text).ok()),
        "createdAt": row.try_get::<String, _>("created_at").unwrap_or_default(),
        "completedAt": row.try_get::<Option<String>, _>("completed_at").ok().flatten(),
    });
    StoredOperation {
        operation,
        action: row
            .try_get::<String, _>("action")
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .unwrap_or(Value::Null),
        request_id: row.try_get::<String, _>("request_id").unwrap_or_default(),
    }
}

fn db(error: sqlx::Error) -> ApiError {
    tracing::error!(error = %error, "控制面数据库错误");
    ApiError::internal("控制面存储不可用")
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .is_some_and(|error| error.is_unique_violation())
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn optional_text(value: &Value, key: &str) -> Option<String> {
    match value.get(key)? {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        other => Some(other.to_string()),
    }
}

fn number(value: &Value, key: &str) -> f64 {
    value.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

fn flag(value: &Value, key: &str) -> i64 {
    i64::from(value.get(key).and_then(Value::as_bool).unwrap_or(false))
}
