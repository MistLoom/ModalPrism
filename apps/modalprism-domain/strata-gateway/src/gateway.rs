//! 门面：业务只和这里打交道。
//!
//! 写入分两步：`apply` 只落版本化行，`flush` 钉住快照、标为可见并触发派生。
//! 读取按快照还原，查询走类型化 `Request`；Blob 读写按内容寻址与区间定位。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use strata_model::{
    AccessTask, ApplyOpts, ChangeSet, CommitResult, PartRef, QueueName, TaskId, Timestamp, WaitFor,
};
use tokio::sync::Mutex;

use crate::blob::{self, BlobService, PreparedBlob};
use crate::commit;
use crate::config::Options;
use crate::data::{self, Batch, Dataset, SnapshotRow};
use crate::db::{Db, Tx};
use crate::derive::{self, DerivationTask};
use crate::error::{Error, Result};
use crate::index::IndexStore;
use crate::query::{self, Request, Response};
use crate::schema;
use crate::sql::{Dialect, Sql};
use crate::staging::Staging;
use crate::task;

pub struct Gateway {
    root: PathBuf,
    options: Options,
    db: Arc<Db>,
    blobs: BlobService,
    index: IndexStore,
    staging: Staging,
    /// 串行化接收与发布，保证快照只包含完整批次。
    write_gate: Mutex<()>,
}

/// 物理清理结果。外部对象删除失败时 `orphanedObjects` 保留待重试数量。
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurgeReport {
    pub tables: BTreeMap<String, u64>,
    pub objects: u64,
    pub orphaned_objects: u64,
}

const CONTENT_TABLES: &[&str] = &[
    "g_resources",
    "g_resource_marks",
    "g_facets",
    "g_blobs",
    "g_relations",
    "g_identities",
    "g_state",
    "g_snapshots",
    "g_commits",
    "g_hot",
];

impl Gateway {
    pub async fn open(root: impl AsRef<Path>, options: Options) -> Result<Self> {
        options.validate()?;
        let root = root.as_ref().to_path_buf();
        tokio::fs::create_dir_all(&root).await?;
        let db = Arc::new(Db::open(&options.backend, &root).await?);
        schema::migrate(&db, options.text.tokenizer).await?;
        let store = blob::open_store(&options.blobs, &root).await?;
        let blobs = BlobService::new(store, options.inline_threshold, options.dedicated_threshold)?;
        let index = IndexStore::open(Arc::clone(&db), options.text.tokenizer).await?;
        let staging = Staging::open(&root).await?;
        commit::recover(&db, &staging, options.staging_ttl).await?;
        Ok(Self {
            root,
            options,
            db,
            blobs,
            index,
            staging,
            write_gate: Mutex::new(()),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn options(&self) -> &Options {
        &self.options
    }

    pub fn blobs(&self) -> &BlobService {
        &self.blobs
    }

    /// 是否检测到 sqlite-vec 加速。
    pub fn vector_accelerated(&self) -> bool {
        self.index.sqlite_vec()
    }

    /// 元数据与 Blob 后端名字，供启动日志。
    pub fn backend_names(&self) -> (&'static str, &'static str) {
        (self.options.backend.name(), self.options.blobs.name())
    }

    /// 写入一批行与索引。返回本次递增的版本。发布前不可见。
    pub async fn write(&self, mut batch: Batch) -> Result<BTreeMap<String, u64>> {
        data::write(&self.db, &mut batch).await
    }

    pub async fn versions(&self) -> Result<BTreeMap<String, u64>> {
        data::versions(&self.db).await
    }

    pub async fn snapshots(&self) -> Result<Vec<SnapshotRow>> {
        data::snapshots(&self.db).await
    }

    pub async fn snapshot(&self, as_of: Option<u64>) -> Result<Option<SnapshotRow>> {
        match as_of {
            Some(seq) => Ok(Some(
                data::snapshots(&self.db)
                    .await?
                    .into_iter()
                    .find(|row| row.seq == seq)
                    .ok_or(Error::MissingSnapshot(seq))?,
            )),
            None => data::latest_snapshot(&self.db).await,
        }
    }

    /// 暂存：字节落到暂存区，返回可放进 `Body::Staged` 的引用。
    pub async fn stage(&self, bytes: &[u8]) -> Result<PartRef> {
        self.staging.write(bytes).await
    }

    /// 按阈值决定内联还是落对象存储；用于立即写入的 blob。
    pub async fn prepare(&self, bytes: &[u8]) -> Result<PreparedBlob> {
        self.blobs.prepare(bytes).await
    }

    /// 接收变更集。默认只到 `Accepted`；要求可见时顺带发布。
    pub async fn apply(&self, changeset: ChangeSet, opts: ApplyOpts) -> Result<CommitResult> {
        let result = {
            let _gate = self.write_gate.lock().await;
            commit::accept(
                &self.db,
                &self.blobs,
                &self.staging,
                &self.options,
                &changeset,
                Timestamp::now(),
            )
            .await?
        };
        if opts.wait != WaitFor::Visible || result.snapshot.is_some() {
            return Ok(result);
        }
        let flushed = match opts.timeout {
            Some(timeout) => match tokio::time::timeout(timeout, self.flush()).await {
                Ok(Ok(seq)) => seq,
                Ok(Err(_)) => return Ok(result),
                Err(_) => return Err(Error::WaitTimeout(result.commit_id.to_string())),
            },
            None => match self.flush().await {
                Ok(seq) => seq,
                Err(_) => return Ok(result),
            },
        };
        if let Some(seq) = flushed {
            return Ok(CommitResult {
                status: WaitFor::Visible,
                snapshot: Some(seq),
                ..result
            });
        }
        if let Some(found) = data::find_commit(&self.db, result.commit_id.as_str()).await? {
            if found.snapshot_seq.is_some() {
                return Ok(CommitResult {
                    commit_id: result.commit_id,
                    status: WaitFor::Visible,
                    snapshot: found.snapshot_seq,
                    duplicate: result.duplicate,
                });
            }
        }
        Err(Error::NotVisible(result.commit_id.to_string()))
    }

    /// 发布所有已接收提交，返回快照序号；没有待发布提交时返回 `None`。
    pub async fn flush(&self) -> Result<Option<u64>> {
        let _gate = self.write_gate.lock().await;
        let db = &*self.db;
        commit::flush(
            db,
            &self.staging,
            &self.options,
            async |snapshot: &SnapshotRow| commit::trigger(db, &self.options, snapshot).await,
        )
        .await
    }

    pub async fn recover(&self) -> Result<()> {
        let _gate = self.write_gate.lock().await;
        commit::recover(&self.db, &self.staging, self.options.staging_ttl).await
    }

    /// 物理清空全部内容、索引、快照与内容 Blob。
    ///
    /// 外部对象先按引用清单登记到 `g_meta`，删除失败时保留待重试清单；
    /// 重试会再次读取清单，保证对象删除幂等且不依赖已删除的元数据。
    pub async fn purge_content(&self) -> Result<PurgeReport> {
        let _gate = self.write_gate.lock().await;
        let dialect = self.db.dialect();
        let objects = self.pending_objects().await?;
        let mut report = PurgeReport::default();
        let mut tx = self.db.begin().await?;
        let mut tables: Vec<String> = CONTENT_TABLES
            .iter()
            .map(|value| (*value).to_string())
            .collect();
        match dialect {
            Dialect::Sqlite => {
                tables.push("g_vectors".into());
                tables.push("g_fts".into());
            }
            Dialect::Postgres => tables.push("g_docs".into()),
        }
        tables.extend(vector_tables(&mut tx).await?);
        for table in &tables {
            let removed = delete_table(&mut tx, table).await?;
            report.tables.insert(table.clone(), removed);
        }
        let mut counters = Sql::new(dialect);
        counters.push("DELETE FROM g_counters");
        tx.exec(&counters).await?;
        let mut versions = Sql::new(dialect);
        versions.push("UPDATE g_versions SET version = 0");
        tx.exec(&versions).await?;
        set_orphan_list(&mut tx, &objects).await?;
        tx.commit().await?;

        let removed = self.remove_objects(&objects).await?;
        report.objects = removed;
        report.orphaned_objects = objects.len().saturating_sub(removed as usize) as u64;
        Ok(report)
    }

    /// 物理删除不再被任何 facet 引用的 blob 行与外部对象。
    pub async fn purge_unreferenced_blobs(&self, dry_run: bool) -> Result<PurgeReport> {
        let _gate = self.write_gate.lock().await;
        let mut report = PurgeReport::default();
        let mut sql = Sql::new(self.db.dialect());
        sql.push(
            "SELECT sha256, tier FROM g_blobs b WHERE NOT EXISTS (SELECT 1 FROM g_facets f WHERE f.blob_ref = b.sha256)",
        );
        let rows = self.db.fetch(&sql).await?;
        report.tables.insert("g_blobs".into(), rows.len() as u64);
        let mut objects = Vec::new();
        for row in &rows {
            if row.text("tier")? != "inline" {
                objects.push(row.text("sha256")?);
            }
        }
        report.objects = objects.len() as u64;
        if dry_run || rows.is_empty() {
            return Ok(report);
        }
        let mut orphans = self.orphan_list().await?;
        orphans.extend(objects.iter().cloned());
        orphans.sort();
        orphans.dedup();
        let mut tx = self.db.begin().await?;
        let mut delete = Sql::new(tx.dialect());
        delete.push(
            "DELETE FROM g_blobs WHERE NOT EXISTS (SELECT 1 FROM g_facets f WHERE f.blob_ref = g_blobs.sha256)",
        );
        tx.exec(&delete).await?;
        set_orphan_list(&mut tx, &orphans).await?;
        tx.commit().await?;
        let removed = self.remove_objects(&objects).await?;
        report.objects = removed;
        report.orphaned_objects = objects.len().saturating_sub(removed as usize) as u64;
        Ok(report)
    }

    /// 物理删除一批资源：行、标记、facet、关系、身份与派生索引；
    /// 不再被任何 facet 引用的 Blob 会同时从元数据与对象存储移除。
    pub async fn purge_resources(&self, rids: &[String]) -> Result<PurgeReport> {
        let _gate = self.write_gate.lock().await;
        let mut report = PurgeReport::default();
        let mut rids: Vec<String> = rids
            .iter()
            .filter(|rid| !rid.trim().is_empty())
            .cloned()
            .collect();
        rids.sort();
        rids.dedup();
        if rids.is_empty() {
            return Ok(report);
        }
        let dialect = self.db.dialect();
        let mut candidates = Sql::new(dialect);
        candidates.push(
            "SELECT DISTINCT f.blob_ref AS sha, b.tier AS tier FROM g_facets f JOIN g_blobs b ON b.sha256 = f.blob_ref WHERE f.blob_ref IS NOT NULL AND ",
        );
        candidates.in_list("f.rid", &rids);
        let mut objects = Vec::new();
        for row in self.db.fetch(&candidates).await? {
            if row.text("tier")? != "inline" {
                objects.push(row.text("sha")?);
            }
        }
        let mut tx = self.db.begin().await?;
        let mut facets = Sql::new(dialect);
        facets.push("DELETE FROM g_facets WHERE ");
        facets.in_list("rid", &rids);
        report
            .tables
            .insert("g_facets".into(), tx.exec(&facets).await?);
        let mut relations = Sql::new(dialect);
        relations.push("DELETE FROM g_relations WHERE ");
        relations.in_list("src_rid", &rids);
        relations.push(" OR ");
        relations.in_list("dst_rid", &rids);
        report
            .tables
            .insert("g_relations".into(), tx.exec(&relations).await?);
        let mut marks = Sql::new(dialect);
        marks.push("DELETE FROM g_resource_marks WHERE ");
        marks.in_list("rid", &rids);
        report
            .tables
            .insert("g_resource_marks".into(), tx.exec(&marks).await?);
        let mut resources = Sql::new(dialect);
        resources.push("DELETE FROM g_resources WHERE ");
        resources.in_list("rid", &rids);
        report
            .tables
            .insert("g_resources".into(), tx.exec(&resources).await?);
        let mut identities = Sql::new(dialect);
        identities.push("DELETE FROM g_identities WHERE ");
        identities.in_list("rid", &rids);
        report
            .tables
            .insert("g_identities".into(), tx.exec(&identities).await?);
        let mut index_tables = Vec::new();
        match dialect {
            Dialect::Sqlite => {
                index_tables.push("g_vectors".to_string());
                index_tables.push("g_fts".to_string());
            }
            Dialect::Postgres => index_tables.push("g_docs".to_string()),
        }
        index_tables.extend(vector_tables(&mut tx).await?);
        for table in &index_tables {
            let mut sql = Sql::new(dialect);
            sql.push("DELETE FROM ").push(table).push(" WHERE ");
            sql.in_list("rid", &rids);
            report.tables.insert(table.clone(), tx.exec(&sql).await?);
        }
        let mut blobs = Sql::new(dialect);
        blobs.push(
            "DELETE FROM g_blobs WHERE NOT EXISTS (SELECT 1 FROM g_facets f WHERE f.blob_ref = g_blobs.sha256)",
        );
        report
            .tables
            .insert("g_blobs".into(), tx.exec(&blobs).await?);
        objects.sort();
        objects.dedup();
        let mut external = Vec::new();
        if !objects.is_empty() {
            let mut remaining = Sql::new(dialect);
            remaining.push("SELECT sha256 FROM g_blobs WHERE ");
            remaining.in_list("sha256", &objects);
            let mut kept = Vec::new();
            for row in tx.fetch(&remaining).await? {
                kept.push(row.text("sha256")?);
            }
            external.extend(
                objects
                    .iter()
                    .filter(|sha| !kept.iter().any(|kept| kept == *sha))
                    .cloned(),
            );
        }
        tx.commit().await?;
        if !external.is_empty() {
            let mut orphans = self.orphan_list().await?;
            orphans.extend(external.iter().cloned());
            orphans.sort();
            orphans.dedup();
            let mut tx = self.db.begin().await?;
            set_orphan_list(&mut tx, &orphans).await?;
            tx.commit().await?;
            report.objects = self.remove_objects(&external).await?;
        }
        report.orphaned_objects = external.len().saturating_sub(report.objects as usize) as u64;
        Ok(report)
    }

    /// 外部对象清单：`g_blobs` 中非内联层级的对象。
    async fn external_objects(&self) -> Result<Vec<String>> {
        let mut sql = Sql::new(self.db.dialect());
        sql.push("SELECT sha256, tier FROM g_blobs WHERE tier <> 'inline'");
        Ok(self
            .db
            .fetch(&sql)
            .await?
            .into_iter()
            .filter_map(|row| row.text("sha256").ok())
            .collect())
    }

    async fn orphan_list(&self) -> Result<Vec<String>> {
        let mut sql = Sql::new(self.db.dialect());
        sql.push("SELECT value FROM g_meta WHERE key = ");
        sql.text("purge_orphans");
        let Some(row) = self.db.fetch_one(&sql).await? else {
            return Ok(Vec::new());
        };
        let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&row.text("value")?) else {
            return Ok(Vec::new());
        };
        Ok(items
            .into_iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect())
    }

    /// 删除外部对象；失败时保留孤儿清单，成功时从清单中移除。
    async fn remove_objects(&self, objects: &[String]) -> Result<u64> {
        if objects.is_empty() {
            return Ok(0);
        }
        for sha in objects {
            self.blobs.remove(sha).await?;
        }
        let mut orphans = self.orphan_list().await?;
        orphans.retain(|sha| !objects.iter().any(|item| item == sha));
        let mut tx = self.db.begin().await?;
        set_orphan_list(&mut tx, &orphans).await?;
        tx.commit().await?;
        Ok(objects.len() as u64)
    }

    /// 只清派生 facet、全文与向量索引，保留结构化内容与关系。
    pub async fn purge_derived(&self) -> Result<PurgeReport> {
        let _gate = self.write_gate.lock().await;
        let dialect = self.db.dialect();
        let mut report = PurgeReport::default();
        let mut tx = self.db.begin().await?;
        let mut facets = Sql::new(dialect);
        facets.push("DELETE FROM g_facets WHERE kind <> 'struct'");
        report
            .tables
            .insert("g_facets".into(), tx.exec(&facets).await?);
        let mut tables = Vec::new();
        match dialect {
            Dialect::Sqlite => {
                tables.push("g_vectors".to_string());
                tables.push("g_fts".to_string());
            }
            Dialect::Postgres => tables.push("g_docs".to_string()),
        }
        tables.extend(vector_tables(&mut tx).await?);
        for table in &tables {
            let removed = delete_table(&mut tx, table).await?;
            report.tables.insert(table.clone(), removed);
        }
        tx.commit().await?;
        Ok(report)
    }

    /// 当前待删除的外部对象：已登记的孤儿清单加上 `g_blobs` 中非内联对象。
    async fn pending_objects(&self) -> Result<Vec<String>> {
        let dialect = self.db.dialect();
        let mut result = Vec::new();
        let mut stored = Sql::new(dialect);
        stored.push("SELECT value FROM g_meta WHERE key = ");
        stored.text("purge_orphans");
        if let Some(row) = self.db.fetch_one(&stored).await?
            && let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&row.text("value")?)
        {
            result.extend(
                items
                    .into_iter()
                    .filter_map(|item| item.as_str().map(str::to_string)),
            );
        }
        let mut blobs = Sql::new(dialect);
        blobs.push("SELECT sha256, tier FROM g_blobs");
        for row in self.db.fetch(&blobs).await? {
            if row.text("tier")? != "inline" {
                result.push(row.text("sha256")?);
            }
        }
        result.sort();
        result.dedup();
        Ok(result)
    }

    pub async fn view_latest(&self) -> Result<Dataset> {
        commit::view_latest(&self.db).await
    }

    pub async fn view_snapshot(&self, seq: u64) -> Result<Dataset> {
        commit::view_snapshot(&self.db, seq).await
    }

    pub async fn view_time(&self, time: Timestamp) -> Result<Dataset> {
        commit::view_time(&self.db, time).await
    }

    /// 按快照序号列出可见变更。
    pub async fn changes(&self, after: u64, limit: Option<u32>) -> Result<Vec<SnapshotRow>> {
        commit::changes(&self.db, after, limit).await
    }

    pub async fn read(&self, as_of: Option<u64>) -> Result<Dataset> {
        let snapshot = self.snapshot(as_of).await?;
        let versions = snapshot
            .as_ref()
            .map(|row| row.versions.clone())
            .unwrap_or_default();
        let mut dataset = data::read_at(&self.db, &versions).await?;
        dataset.snapshot = snapshot;
        Ok(dataset)
    }

    pub async fn read_blob(&self, versions: &BTreeMap<String, u64>, sha: &str) -> Result<Vec<u8>> {
        let row = data::blob_row(&self.db, versions, sha).await?;
        self.blobs.read(&row.sha256, row.inline.as_deref()).await
    }

    pub async fn read_blob_range(
        &self,
        versions: &BTreeMap<String, u64>,
        sha: &str,
        start: u64,
        len: u64,
    ) -> Result<Vec<u8>> {
        let row = data::blob_row(&self.db, versions, sha).await?;
        self.blobs
            .range(&row.sha256, row.inline.as_deref(), start, len)
            .await
    }

    pub async fn query(&self, request: Request) -> Result<Response> {
        let snapshot = self.snapshot(request.as_of).await?;
        let versions = snapshot
            .as_ref()
            .map(|row| row.versions.clone())
            .unwrap_or_default();
        query::execute(
            &self.db,
            &self.index,
            &self.options.fusion,
            &versions,
            snapshot.as_ref().map(|row| row.seq),
            &request,
        )
        .await
    }

    /// 按快照触发派生；已满足的声明不会重复入队。
    pub async fn trigger(&self, snapshot: &SnapshotRow) -> Result<Vec<AccessTask>> {
        if self.options.derivations.is_empty() {
            return Ok(Vec::new());
        }
        let dataset = data::read_at(&self.db, &snapshot.versions).await?;
        derive::trigger(&self.db, &self.options.derivations, &dataset, snapshot.seq).await
    }

    /// 解析派生任务载荷。
    pub fn derivation(&self, task: &AccessTask) -> Result<DerivationTask> {
        derive::parse_task(&task.spec)
    }

    pub async fn enqueue(&self, tasks: &[AccessTask]) -> Result<Vec<AccessTask>> {
        task::enqueue(&self.db, tasks).await
    }

    pub async fn lease(
        &self,
        queue: QueueName,
        count: usize,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<Vec<AccessTask>> {
        task::lease(&self.db, queue, count, lease_for, now).await
    }

    /// 只领取派生任务，不动信源调度与跟进任务。
    pub async fn lease_derivations(
        &self,
        queue: QueueName,
        count: usize,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<Vec<AccessTask>> {
        task::lease_derivations(&self.db, queue, count, lease_for, now).await
    }

    pub async fn lease_follows(
        &self,
        queue: QueueName,
        count: usize,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<Vec<AccessTask>> {
        task::lease_follows(&self.db, queue, count, lease_for, now).await
    }

    pub async fn lease_id(
        &self,
        task_id: &TaskId,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<AccessTask> {
        task::lease_id(&self.db, task_id, lease_for, now).await
    }

    pub async fn lease_ready(
        &self,
        source: &str,
        key: &str,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<Option<AccessTask>> {
        task::lease_ready(&self.db, source, key, lease_for, now).await
    }

    pub async fn extend(
        &self,
        task_id: &TaskId,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<()> {
        task::extend(&self.db, task_id, lease_for, now).await
    }

    pub async fn nack(&self, task_id: &TaskId, retry_at: Timestamp, error: &str) -> Result<()> {
        task::nack(&self.db, task_id, retry_at, error).await
    }

    pub async fn ack(&self, task_id: &TaskId) -> Result<()> {
        task::ack(&self.db, task_id).await
    }

    pub async fn release_leases(&self) -> Result<Vec<AccessTask>> {
        task::release_leases(&self.db).await
    }

    pub async fn leased_count(&self, queue: QueueName, now: Timestamp) -> Result<usize> {
        task::leased_count(&self.db, queue, now).await
    }

    pub async fn task_active(&self, source: &str, key: &str) -> Result<bool> {
        task::active(&self.db, source, key).await
    }

    /// 同定位键上活跃任务的 id（不加租约）。
    pub async fn active_task_id(&self, source: &str, key: &str) -> Result<Option<TaskId>> {
        task::active_id(&self.db, source, key).await
    }

    /// 任务一页，供控制台查看队列状态。
    pub async fn list_tasks(
        &self,
        filter: &task::TaskFilter,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<task::TaskPage> {
        task::list_tasks(&self.db, filter, cursor, limit).await
    }

    /// 队列全景统计。
    pub async fn task_stats(&self) -> Result<task::TaskStats> {
        task::stats(&self.db).await
    }

    /// 取消一个尚未终态的任务。
    pub async fn cancel_task(&self, task_id: &TaskId) -> Result<bool> {
        task::cancel(&self.db, task_id).await
    }

    /// 重试或恢复一个任务，让它立即参与下一轮租约。
    pub async fn requeue_task(&self, task_id: &TaskId) -> Result<bool> {
        task::requeue(&self.db, task_id).await
    }

    /// 删除一个终态任务。
    pub async fn remove_task(&self, task_id: &TaskId) -> Result<bool> {
        task::remove(&self.db, task_id).await
    }

    /// 按条件清理终态任务。
    pub async fn purge_tasks(&self, filter: &task::TaskFilter) -> Result<u64> {
        task::purge(&self.db, filter).await
    }

    /// 按条件计数，供分页页脚与清理确认。
    pub async fn count_tasks(&self, filter: &task::TaskFilter) -> Result<u64> {
        task::count_tasks(&self.db, filter).await
    }

    /// 每队列 × 状态计数。
    pub async fn queue_counts(&self) -> Result<Vec<task::QueueCounts>> {
        task::queue_counts(&self.db).await
    }

    /// 读取一条任务的完整内容，供装配层执行。
    pub async fn read_task(&self, task_id: &TaskId) -> Result<Option<task::StoredTask>> {
        task::read(&self.db, task_id).await
    }

    /// 单条任务行，供新建后回显。
    pub async fn task_row(&self, task_id: &TaskId) -> Result<Option<task::TaskRow>> {
        task::row(&self.db, task_id).await
    }

    /// 已接收状态的最新值与版本；不要求已发布。
    pub async fn accepted_state(&self, scope: &str, key: &str) -> Result<Option<(u64, String)>> {
        data::accepted_state(&self.db, scope, key).await
    }

    pub async fn hot_get(&self, kind: &str, key: &str) -> Result<Option<String>> {
        task::hot_get(&self.db, kind, key).await
    }

    pub async fn hot_put(&self, kind: &str, key: &str, value: &str) -> Result<()> {
        task::hot_put(&self.db, kind, key, value).await
    }

    /// 按保留计划裁剪过期行；被裁掉的 blob 行同时删除外部对象。
    pub async fn prune(
        &self,
        plan: &crate::maintenance::PrunePlan,
        dry_run: bool,
    ) -> Result<crate::maintenance::PruneReport> {
        let before = if dry_run {
            Vec::new()
        } else {
            self.external_objects().await?
        };
        let report = crate::maintenance::prune(&self.db, plan, dry_run).await?;
        if dry_run || report.blobs == 0 {
            return Ok(report);
        }
        let remaining = self.external_objects().await?;
        let doomed: Vec<String> = before
            .into_iter()
            .filter(|sha| !remaining.contains(sha))
            .collect();
        if !doomed.is_empty() {
            let mut orphans = self.orphan_list().await?;
            orphans.extend(doomed.iter().cloned());
            orphans.sort();
            orphans.dedup();
            let mut tx = self.db.begin().await?;
            set_orphan_list(&mut tx, &orphans).await?;
            tx.commit().await?;
            self.remove_objects(&doomed).await?;
        }
        Ok(report)
    }

    /// 各表行数。
    pub async fn store_counts(&self) -> Result<crate::maintenance::StoreCounts> {
        crate::maintenance::counts(&self.db).await
    }

    /// 测试/运维入口。
    pub async fn count(&self, logical: &str) -> Result<u64> {
        data::count(&self.db, logical).await
    }

    pub fn db(&self) -> &Db {
        &self.db
    }
}

async fn delete_table(tx: &mut Tx<'_>, table: &str) -> Result<u64> {
    let mut sql = Sql::new(tx.dialect());
    sql.push("DELETE FROM ").push(table);
    tx.exec(&sql).await
}

async fn vector_tables(tx: &mut Tx<'_>) -> Result<Vec<String>> {
    let dialect = tx.dialect();
    let statement = match dialect {
        Dialect::Sqlite => {
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE 'g_vec_%'"
        }
        Dialect::Postgres => {
            "SELECT tablename AS name FROM pg_tables WHERE schemaname = current_schema() AND tablename LIKE 'g_vec_%'"
        }
    };
    let rows = tx.fetch(&Sql::raw(dialect, statement)).await?;
    Ok(rows
        .iter()
        .filter_map(|row| row.text("name").ok())
        .filter(|name| {
            name.starts_with("g_vec_")
                && name[6..].chars().all(|value| value.is_ascii_digit())
                && name.len() > 6
        })
        .collect())
}

async fn set_orphan_list(tx: &mut Tx<'_>, objects: &[String]) -> Result<()> {
    let dialect = tx.dialect();
    let mut clear = Sql::new(dialect);
    clear.push("DELETE FROM g_meta WHERE key = ");
    clear.text("purge_orphans");
    tx.exec(&clear).await?;
    if objects.is_empty() {
        return Ok(());
    }
    let mut insert = Sql::new(dialect);
    insert.push("INSERT INTO g_meta(key, value) VALUES (");
    insert.text("purge_orphans");
    insert.push(", ");
    insert.text(serde_json::to_string(objects).unwrap_or_default());
    insert.push(")");
    tx.exec(&insert).await?;
    Ok(())
}
