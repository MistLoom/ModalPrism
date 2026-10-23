//! 门面：业务只和这里打交道。
//!
//! 写入分两步：`apply` 只落版本化行，`flush` 钉住快照、标为可见并触发派生。
//! 读取按快照还原，查询走类型化 `Request`；Blob 读写按内容寻址与区间定位。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use strata_model::{
    AccessTask, ApplyOpts, ChangeSet, CommitResult, PartRef, QueueName, TaskId, Timestamp, WaitFor,
};
use tokio::sync::Mutex;

use crate::blob::{self, BlobService, PreparedBlob};
use crate::commit;
use crate::config::Options;
use crate::data::{self, Batch, Dataset, SnapshotRow};
use crate::db::Db;
use crate::derive::{self, DerivationTask};
use crate::error::{Error, Result};
use crate::index::IndexStore;
use crate::query::{self, Request, Response};
use crate::schema;
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

    /// 任务列表，供控制台查看队列状态。
    pub async fn list_tasks(
        &self,
        queue: Option<QueueName>,
        status: Option<&str>,
        limit: usize,
    ) -> Result<Vec<task::TaskRow>> {
        task::list_tasks(&self.db, queue, status, limit).await
    }

    /// 手动重试一个排队中的任务。
    pub async fn retry_task(&self, task_id: &TaskId) -> Result<bool> {
        task::retry_task(&self.db, task_id).await
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

    /// 测试/运维入口。
    pub async fn count(&self, logical: &str) -> Result<u64> {
        data::count(&self.db, logical).await
    }

    pub fn db(&self) -> &Db {
        &self.db
    }
}
