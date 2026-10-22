//! 门面：业务只和这里打交道。
//!
//! 一、写入走 `write`，按表递增版本；二、`publish` 钉住快照并把派生任务入队；
//! 三、读取按快照还原；四、查询走类型化 `Request`。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use strata_model::{AccessTask, QueueName, TaskId, Timestamp};

use crate::blob::{self, BlobService, PreparedBlob};
use crate::config::Options;
use crate::data::{self, Batch, Dataset, SnapshotRow};
use crate::db::Db;
use crate::derive::{self, DerivationTask};
use crate::error::{Error, Result};
use crate::index::IndexStore;
use crate::query::{self, Request, Response};
use crate::schema;
use crate::task;

pub struct Gateway {
    root: PathBuf,
    options: Options,
    db: Arc<Db>,
    blobs: BlobService,
    index: IndexStore,
}

impl Gateway {
    pub async fn open(root: impl AsRef<Path>, options: Options) -> Result<Self> {
        options.validate()?;
        let root = root.as_ref().to_path_buf();
        tokio::fs::create_dir_all(&root).await?;
        let db = Arc::new(Db::open(&options.backend, &root).await?);
        schema::migrate(&db, options.text.tokenizer).await?;
        let store = blob::open_store(&options.blobs, &root).await?;
        let blobs = BlobService::new(
            store,
            options.inline_threshold,
            options.dedicated_threshold,
        )?;
        let index = IndexStore::open(Arc::clone(&db), options.text.tokenizer).await?;
        Ok(Self {
            root,
            options,
            db,
            blobs,
            index,
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

    /// 写一批行与索引。返回本次递增的版本。
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
        let snapshots = data::snapshots(&self.db).await?;
        match as_of {
            Some(seq) => snapshots
                .into_iter()
                .find(|row| row.seq == seq)
                .map(Some)
                .ok_or(Error::MissingSnapshot(seq)),
            None => Ok(snapshots.into_iter().max_by_key(|row| row.seq)),
        }
    }

    /// 发布快照，返回钉住的版本组合。
    pub async fn publish(
        &self,
        commit_ids: &[String],
        kind: &str,
        config_generation: u64,
    ) -> Result<SnapshotRow> {
        let published_at = Timestamp::now().to_rfc3339()?;
        data::write_snapshot(
            &self.db,
            commit_ids,
            kind,
            config_generation,
            &published_at,
        )
        .await
    }

    /// 发布快照并触发派生。
    pub async fn publish_and_trigger(
        &self,
        commit_ids: &[String],
        kind: &str,
        config_generation: u64,
    ) -> Result<(SnapshotRow, Vec<AccessTask>)> {
        let snapshot = self.publish(commit_ids, kind, config_generation).await?;
        let tasks = self.trigger(&snapshot).await?;
        Ok((snapshot, tasks))
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

    /// 暂存：按阈值决定内联还是落对象存储。
    pub async fn stage(&self, bytes: &[u8]) -> Result<PreparedBlob> {
        self.blobs.prepare(bytes).await
    }

    pub async fn read_blob(
        &self,
        versions: &BTreeMap<String, u64>,
        sha: &str,
    ) -> Result<Vec<u8>> {
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

    /// 按快照触发派生。
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

    pub async fn nack(
        &self,
        task_id: &TaskId,
        retry_at: Timestamp,
        error: &str,
    ) -> Result<()> {
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

    pub async fn hot_get(&self, kind: &str, key: &str) -> Result<Option<String>> {
        task::hot_get(&self.db, kind, key).await
    }

    pub async fn hot_put(&self, kind: &str, key: &str, value: &str) -> Result<()> {
        task::hot_put(&self.db, kind, key, value).await
    }

    /// 测试/运维入口。
    pub async fn count(&self, logical: &str) -> Result<u64> {
        data::count(&self.db, logical).await
    }

    pub fn db(&self) -> &Db {
        &self.db
    }
}
