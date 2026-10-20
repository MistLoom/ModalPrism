//! 打开目录、接收变更集、刷写快照与恢复。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use strata_model::{
    ApplyOpts, Body, ChangeSet, CommitId, CommitResult, Op, PartRef, QueueName, TaskId, Timestamp,
    WaitFor, content_hash,
};
use tokio::sync::Mutex;
use ulid::Ulid;

use crate::control::{ControlStore, FlushBatch, SqliteControl};
use crate::error::Error;
use crate::lance_io::{
    BlobWrite, DataRows, FacetWrite, IdentityWrite, LanceStore, MarkWrite, RelationWrite,
    ResourceWrite, SnapshotWrite, StateWrite, TABLES,
};
use crate::records::{FacetRecord, MarkRecord, SnapshotInfo, View};
use crate::resolve::{StoredBody, StoredCommit, StoredOp};
use crate::staging::Staging;

pub const INLINE_TEXT_MAX: u64 = 4 * 1024 * 1024;
pub const BLOB_INLINE_THRESHOLD: usize = 64 * 1024;
pub const BLOB_DEDICATED_THRESHOLD: usize = 16 * 1024 * 1024;
pub const STAGING_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Debug)]
pub struct Options {
    pub inline_text_max: u64,
    pub blob_inline_threshold: usize,
    pub blob_dedicated_threshold: usize,
    pub staging_ttl: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            inline_text_max: INLINE_TEXT_MAX,
            blob_inline_threshold: BLOB_INLINE_THRESHOLD,
            blob_dedicated_threshold: BLOB_DEDICATED_THRESHOLD,
            staging_ttl: STAGING_TTL,
        }
    }
}

struct Inner {
    #[cfg_attr(not(test), allow(dead_code))]
    root: PathBuf,
    options: Options,
    lance: LanceStore,
    control: SqliteControl,
    staging: Staging,
    observed: Option<SnapshotInfo>,
    #[cfg(test)]
    flush_hold: Option<Arc<tokio::sync::Notify>>,
    #[cfg(test)]
    flush_done: Option<Arc<tokio::sync::Notify>>,
}

pub struct Engine {
    inner: Arc<Mutex<Inner>>,
}

impl Engine {
    pub async fn open(root: impl AsRef<Path>, options: Options) -> Result<Self, Error> {
        check_options(&options)?;
        let root = root.as_ref();
        tokio::fs::create_dir_all(root).await?;
        let root = tokio::fs::canonicalize(root).await?;
        let lance_dir = root.join("lance");
        let staging_dir = root.join("staging");
        tokio::fs::create_dir_all(&lance_dir).await?;
        tokio::fs::create_dir_all(&staging_dir).await?;
        let control = SqliteControl::open(&root.join("control.db")).await?;
        let lance = LanceStore::open(
            &lance_dir,
            crate::lance_io::BlobThresholds {
                inline: options.blob_inline_threshold,
                dedicated: options.blob_dedicated_threshold,
            },
        )
        .await?;
        for name in TABLES {
            if control.baseline(name).await?.is_none() {
                let version = lance.earliest(name).await?;
                control.set_baseline(name, version).await?;
            }
        }
        lance.ensure_blob_column().await?;
        let mut inner = Inner {
            root,
            options,
            lance,
            control,
            staging: Staging::new(staging_dir),
            observed: None,
            #[cfg(test)]
            flush_hold: None,
            #[cfg(test)]
            flush_done: None,
        };
        recover(&mut inner).await?;
        Ok(Self {
            inner: Arc::new(Mutex::new(inner)),
        })
    }

    pub async fn stage(&self, bytes: &[u8]) -> Result<PartRef, Error> {
        let inner = self.inner.lock().await;
        inner.staging.write(bytes).await
    }

    pub async fn apply(
        &self,
        changeset: ChangeSet,
        opts: ApplyOpts,
    ) -> Result<CommitResult, Error> {
        let result = {
            let inner = self.inner.lock().await;
            changeset.validate()?;
            for op in &changeset.ops {
                if let Op::PutFacet {
                    body: Body::Staged { part },
                    ..
                } = op
                {
                    inner.staging.check(part).await?;
                }
            }
            let commit_id = CommitId::new(Ulid::new().to_string())?;
            inner
                .control
                .accept(&changeset, &commit_id, Timestamp::now())
                .await?
        };
        if opts.wait != WaitFor::Visible || result.snapshot.is_some() {
            return Ok(result);
        }
        let flushed = match opts.timeout {
            Some(timeout) => {
                let shared = Arc::clone(&self.inner);
                let handle = tokio::spawn(async move { Engine::flush_owned(shared, false).await });
                match tokio::time::timeout(timeout, handle).await {
                    Ok(Ok(Ok(seq))) => seq,
                    Ok(Ok(Err(_))) | Ok(Err(_)) => return Ok(result),
                    Err(_) => return Err(Error::WaitTimeout(result.commit_id.to_string())),
                }
            }
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
        let inner = self.inner.lock().await;
        if let Some(found) = inner.control.outcome(result.commit_id.as_str()).await? {
            if found.snapshot.is_some() {
                return Ok(CommitResult {
                    commit_id: result.commit_id,
                    status: WaitFor::Visible,
                    snapshot: found.snapshot,
                    duplicate: result.duplicate,
                });
            }
        }
        Err(Error::NotVisible(result.commit_id.to_string()))
    }

    pub async fn flush(&self) -> Result<Option<u64>, Error> {
        Self::flush_owned(Arc::clone(&self.inner), false).await
    }

    pub async fn recover(&self) -> Result<(), Error> {
        let mut inner = self.inner.lock().await;
        recover(&mut inner).await
    }

    pub async fn view_latest(&self) -> Result<View, Error> {
        let mut inner = self.inner.lock().await;
        view_latest(&mut inner).await
    }

    pub async fn view_snapshot(&self, seq: u64) -> Result<View, Error> {
        let inner = self.inner.lock().await;
        let info = inner
            .lance
            .snapshot_rows()
            .await?
            .into_iter()
            .find(|row| row.seq == seq)
            .ok_or(Error::MissingSnapshot(seq))?;
        load_view(&inner, info).await
    }

    pub async fn view_time(&self, time: Timestamp) -> Result<View, Error> {
        let inner = self.inner.lock().await;
        let mut best: Option<SnapshotInfo> = None;
        for row in inner.lance.snapshot_rows().await? {
            let published = Timestamp::parse(&row.published_at)?;
            if published <= time && best.as_ref().is_none_or(|current| row.seq > current.seq) {
                best = Some(row);
            }
        }
        match best {
            Some(info) => load_view(&inner, info).await,
            None => Ok(View::default()),
        }
    }

    pub async fn changes(
        &self,
        after: u64,
        limit: Option<u32>,
    ) -> Result<Vec<SnapshotInfo>, Error> {
        let mut inner = self.inner.lock().await;
        let _ = view_latest(&mut inner).await?;
        let seen = inner.observed.as_ref().map(|row| row.seq).unwrap_or(0);
        let mut rows: Vec<_> = inner
            .lance
            .snapshot_rows()
            .await?
            .into_iter()
            .filter(|row| row.seq > after && row.seq <= seen)
            .collect();
        rows.sort_by_key(|row| row.seq);
        if let Some(limit) = limit {
            rows.truncate(limit as usize);
        }
        Ok(rows)
    }

    pub async fn read_blob(
        &self,
        versions: &BTreeMap<String, u64>,
        sha: &str,
    ) -> Result<Vec<u8>, Error> {
        let inner = self.inner.lock().await;
        inner.lance.blob_bytes(versions, sha).await
    }

    pub async fn read_blob_range(
        &self,
        versions: &BTreeMap<String, u64>,
        sha: &str,
        start: u64,
        len: u64,
    ) -> Result<Vec<u8>, Error> {
        let inner = self.inner.lock().await;
        inner.lance.blob_range(versions, sha, start, len).await
    }

    pub async fn enqueue(&self, tasks: &[strata_model::AccessTask]) -> Result<(), Error> {
        let inner = self.inner.lock().await;
        inner.control.enqueue(tasks, Timestamp::now()).await
    }

    pub async fn lease(
        &self,
        queue: QueueName,
        count: usize,
        lease_for: Duration,
    ) -> Result<Vec<strata_model::AccessTask>, Error> {
        let inner = self.inner.lock().await;
        inner
            .control
            .lease(queue, count, lease_for, Timestamp::now())
            .await
    }

    pub async fn extend(&self, task_id: &TaskId, lease_for: Duration) -> Result<(), Error> {
        let inner = self.inner.lock().await;
        inner
            .control
            .extend(task_id, lease_for, Timestamp::now())
            .await
    }

    pub async fn nack(
        &self,
        task_id: &TaskId,
        retry_at: Timestamp,
        error: &str,
    ) -> Result<(), Error> {
        let inner = self.inner.lock().await;
        inner.control.nack(task_id, retry_at, error).await
    }

    async fn flush_owned(
        shared: Arc<Mutex<Inner>>,
        stop_before_snapshot: bool,
    ) -> Result<Option<u64>, Error> {
        #[cfg(test)]
        let done = {
            let inner = shared.lock().await;
            let hold = inner.flush_hold.clone();
            let done = inner.flush_done.clone();
            drop(inner);
            if let Some(hold) = hold {
                hold.notified().await;
            }
            done
        };
        let mut inner = shared.lock().await;
        recover(&mut inner).await?;
        let commits = inner.control.list_accepted().await?;
        if commits.is_empty() {
            let keep = inner.control.accepted_staging().await?;
            inner
                .staging
                .sweep(&keep, inner.options.staging_ttl)
                .await?;
            #[cfg(test)]
            if let Some(done) = done {
                done.notify_one();
            }
            return Ok(None);
        }
        if inner.control.flush_batch().await?.is_some() {
            return Err(Error::DirtyFlush);
        }
        let commit_ids: Vec<String> = commits
            .iter()
            .map(|commit| commit.commit_id.clone())
            .collect();
        let versions = inner.lance.versions().await?;
        inner
            .control
            .put_flush_batch(&FlushBatch {
                commit_ids: commit_ids.clone(),
                versions,
            })
            .await?;
        let base = planning_view(&inner).await?;
        let now = Timestamp::now().to_rfc3339()?;
        let mut rows = plan(
            &inner.staging,
            &commits,
            &base,
            inner.options.inline_text_max,
            &now,
        )
        .await?;
        assign_seq(&inner.control, &mut rows).await?;
        inner.lance.write_data(&rows).await?;
        if stop_before_snapshot {
            #[cfg(test)]
            if let Some(done) = done.clone() {
                done.notify_one();
            }
            return Ok(None);
        }
        let mut versions = inner.lance.versions().await?;
        let current = versions
            .get("snapshots")
            .copied()
            .ok_or_else(|| Error::VersionBehind("snapshots".into()))?;
        let expected = current + 1;
        versions.insert("snapshots".into(), expected);
        let snapshot_seq = inner.control.alloc_snapshot().await?;
        let generation = commits
            .iter()
            .map(|commit| commit.config_generation)
            .max()
            .unwrap_or(0);
        inner
            .lance
            .write_snapshot(&SnapshotWrite {
                seq: snapshot_seq,
                versions,
                commit_ids: commit_ids.clone(),
                kind: "data".into(),
                config_generation: generation,
                published_at: now,
            })
            .await?;
        let actual = inner
            .lance
            .versions()
            .await?
            .get("snapshots")
            .copied()
            .unwrap_or(0);
        if actual != expected {
            return Err(Error::SnapshotVersion { expected, actual });
        }
        let referenced = inner.control.staging_for(&commit_ids).await?;
        inner
            .control
            .finish_visible(&commit_ids, snapshot_seq)
            .await?;
        let keep = inner.control.accepted_staging().await?;
        for sha in referenced {
            if !keep.contains(&sha) {
                inner.staging.remove(&sha).await?;
            }
        }
        inner
            .staging
            .sweep(&keep, inner.options.staging_ttl)
            .await?;
        inner.observed = None;
        #[cfg(test)]
        if let Some(done) = done {
            done.notify_one();
        }
        Ok(Some(snapshot_seq))
    }

    #[cfg(test)]
    pub async fn testing_count(&self, table: &str) -> Result<usize, Error> {
        let inner = self.inner.lock().await;
        inner.lance.count(table).await
    }

    #[cfg(test)]
    pub async fn testing_pause_flush(
        &self,
        hold: Arc<tokio::sync::Notify>,
        done: Arc<tokio::sync::Notify>,
    ) {
        let mut inner = self.inner.lock().await;
        inner.flush_hold = Some(hold);
        inner.flush_done = Some(done);
    }

    pub async fn testing_stop_before_snapshot(&self) -> Result<(), Error> {
        Self::flush_owned(Arc::clone(&self.inner), true).await?;
        Ok(())
    }

    #[cfg(test)]
    pub async fn testing_rewind_to(&self, seq: u64) -> Result<(), Error> {
        let inner = self.inner.lock().await;
        let info = inner
            .lance
            .snapshot_rows()
            .await?
            .into_iter()
            .find(|row| row.seq == seq)
            .ok_or(Error::MissingSnapshot(seq))?;
        inner.lance.restore_tables(&info.versions).await
    }

    #[cfg(test)]
    pub async fn testing_files(&self) -> Result<Vec<String>, Error> {
        let inner = self.inner.lock().await;
        let mut files = Vec::new();
        walk(
            &inner.root.join("lance"),
            &inner.root.join("lance"),
            &mut files,
        )?;
        files.sort();
        Ok(files)
    }

    #[cfg(test)]
    pub async fn testing_blob(&self, sha: &str) -> Result<Vec<u8>, Error> {
        let mut inner = self.inner.lock().await;
        let view = view_latest(&mut inner).await?;
        let versions = view.snapshot.ok_or(Error::MissingSnapshot(0))?.versions;
        inner.lance.blob_bytes(&versions, sha).await
    }
}

fn check_options(options: &Options) -> Result<(), Error> {
    if options.inline_text_max == 0
        || options.blob_inline_threshold == 0
        || options.blob_dedicated_threshold <= options.blob_inline_threshold
    {
        Err(Error::BlobThreshold)
    } else {
        Ok(())
    }
}

/// 刷写批次还在时，已进入快照的提交只补可见标记；否则把表收回批次前的版本。
async fn recover(inner: &mut Inner) -> Result<(), Error> {
    if let Some(batch) = inner.control.flush_batch().await? {
        match published(&inner.lance.snapshot_rows().await?, &batch.commit_ids)? {
            Some(seq) => inner.control.finish_visible(&batch.commit_ids, seq).await?,
            None => {
                inner.lance.restore_tables(&batch.versions).await?;
                inner.control.clear_flush_batch().await?;
            }
        }
    }
    let current = inner.lance.versions().await?;
    let snaps = inner.lance.snapshot_rows().await?;
    let latest = snaps.iter().max_by_key(|row| row.seq);
    let mut targets = BTreeMap::new();
    for name in TABLES {
        let now = current
            .get(name)
            .copied()
            .ok_or_else(|| Error::VersionBehind((*name).into()))?;
        let target = if let Some(latest) = latest {
            *latest
                .versions
                .get(name)
                .ok_or_else(|| Error::VersionBehind((*name).into()))?
        } else {
            inner
                .control
                .baseline(name)
                .await?
                .ok_or_else(|| Error::VersionBehind((*name).into()))?
        };
        if now < target {
            return Err(Error::VersionBehind((*name).into()));
        }
        if now > target {
            targets.insert((*name).to_string(), target);
        }
    }
    if !targets.is_empty() {
        inner.lance.restore_tables(&targets).await?;
    }
    Ok(())
}

fn published(snaps: &[SnapshotInfo], ids: &[String]) -> Result<Option<u64>, Error> {
    let Some(latest) = snaps.iter().max_by_key(|row| row.seq) else {
        return Ok(None);
    };
    let hit = ids
        .iter()
        .filter(|id| latest.commit_ids.iter().any(|item| item == *id))
        .count();
    if hit == ids.len() {
        Ok(Some(latest.seq))
    } else if hit == 0 {
        Ok(None)
    } else {
        Err(Error::PartialBatch)
    }
}

async fn planning_view(inner: &Inner) -> Result<View, Error> {
    let Some(info) = inner
        .lance
        .snapshot_rows()
        .await?
        .into_iter()
        .max_by_key(|row| row.seq)
    else {
        return Ok(View::default());
    };
    load_view(inner, info).await
}

async fn load_view(inner: &Inner, info: SnapshotInfo) -> Result<View, Error> {
    let mut view = inner.lance.read_at(&info.versions).await?;
    view.snapshot = Some(info);
    Ok(view)
}

async fn view_latest(inner: &mut Inner) -> Result<View, Error> {
    let latest = inner
        .lance
        .snapshot_rows()
        .await?
        .into_iter()
        .max_by_key(|row| row.seq);
    let chosen = match (latest, inner.observed.clone()) {
        (Some(latest), Some(seen)) if seen.seq > latest.seq => seen,
        (Some(latest), _) => latest,
        (None, Some(seen)) => seen,
        (None, None) => return Ok(View::default()),
    };
    if inner
        .observed
        .as_ref()
        .is_none_or(|seen| chosen.seq > seen.seq)
    {
        inner.observed = Some(chosen.clone());
    }
    load_view(inner, chosen).await
}

async fn assign_seq(control: &SqliteControl, rows: &mut DataRows) -> Result<(), Error> {
    let count = (rows.resources.len()
        + rows.marks.len()
        + rows.facets.len()
        + rows.blobs.len()
        + rows.relations.len()
        + rows.identities.len()
        + rows.states.len()) as u64;
    if count == 0 {
        return Ok(());
    }
    let mut seq = control.alloc_seq(count).await?;
    let mut take = || {
        let value = seq;
        seq += 1;
        value
    };
    for row in &mut rows.resources {
        row.seq = take();
    }
    for row in &mut rows.marks {
        row.seq = take();
    }
    for row in &mut rows.facets {
        row.seq = take();
    }
    for row in &mut rows.blobs {
        row.seq = take();
    }
    for row in &mut rows.relations {
        row.seq = take();
    }
    for row in &mut rows.identities {
        row.seq = take();
    }
    for row in &mut rows.states {
        row.seq = take();
    }
    Ok(())
}

struct FacetHead {
    version: u64,
    hash: String,
}

struct MarkHead {
    labels: BTreeMap<String, String>,
    tombstone: bool,
    reason: Option<String>,
}

async fn plan(
    staging: &Staging,
    commits: &[StoredCommit],
    view: &View,
    inline_text_max: u64,
    now: &str,
) -> Result<DataRows, Error> {
    let mut facets = HashMap::<(String, String, String), FacetHead>::new();
    for facet in &view.facets {
        let key = facet_key(facet);
        let head = facets.entry(key).or_insert(FacetHead {
            version: 0,
            hash: String::new(),
        });
        if facet.version >= head.version {
            head.version = facet.version;
            head.hash = facet.content_hash.clone();
        }
    }
    let mut best_marks = HashMap::<String, (u64, MarkHead)>::new();
    for mark in &view.marks {
        let head = mark_head(mark)?;
        match best_marks.get(&mark.rid) {
            Some((seq, _)) if *seq > mark.seq => {}
            _ => {
                best_marks.insert(mark.rid.clone(), (mark.seq, head));
            }
        }
    }
    let mut marks = best_marks
        .into_iter()
        .map(|(rid, (_, head))| (rid, head))
        .collect::<HashMap<_, _>>();
    let mut rows = DataRows {
        resources: Vec::new(),
        marks: Vec::new(),
        facets: Vec::new(),
        blobs: Vec::new(),
        relations: Vec::new(),
        identities: Vec::new(),
        states: Vec::new(),
    };
    let mut blobs_seen = HashSet::new();
    for commit in commits {
        for op in &commit.ops {
            match op {
                StoredOp::Ensure {
                    identity,
                    ns,
                    labels,
                    rid,
                    source_id,
                    created_at,
                    created,
                } => {
                    if !created {
                        continue;
                    }
                    rows.resources.push(ResourceWrite {
                        rid: rid.clone(),
                        ns: ns.clone(),
                        source_id: source_id.clone(),
                        created_at: created_at.clone(),
                        seq: 0,
                    });
                    rows.identities.push(IdentityWrite {
                        identity_key: identity.clone(),
                        rid: rid.clone(),
                        scheme: identity.split(':').next().unwrap_or("").to_string(),
                        commit_id: commit.commit_id.clone(),
                        seq: 0,
                    });
                    if !labels.is_empty() {
                        let head = MarkHead {
                            labels: labels.clone(),
                            tombstone: false,
                            reason: None,
                        };
                        push_mark(&mut rows, rid, &head);
                        marks.insert(rid.clone(), head);
                    }
                }
                StoredOp::PutFacet {
                    rid,
                    kind,
                    variant,
                    body,
                    media_type,
                    inputs,
                    params_hash,
                    content_hash: hash,
                    size,
                } => {
                    let key = (
                        rid.clone(),
                        kind.clone(),
                        variant.clone().unwrap_or_default(),
                    );
                    if facets.get(&key).is_some_and(|head| head.hash == *hash) {
                        continue;
                    }
                    let version = facets.get(&key).map(|head| head.version).unwrap_or(0) + 1;
                    let (body_text, body_struct, blob_ref) = materialize(
                        staging,
                        body,
                        hash,
                        media_type,
                        &mut Spill {
                            rows: &mut rows,
                            seen: &mut blobs_seen,
                            now,
                            inline_text_max,
                        },
                    )
                    .await?;
                    rows.facets.push(FacetWrite {
                        rid: rid.clone(),
                        kind: kind.clone(),
                        variant: variant.clone(),
                        version,
                        content_hash: hash.clone(),
                        media_type: media_type.clone(),
                        size: *size,
                        body_text,
                        body_struct,
                        blob_ref,
                        inputs: inputs.clone(),
                        params_hash: params_hash.clone(),
                        producer_module: commit.producer_module.clone(),
                        producer_version: commit.producer_version.clone(),
                        config_generation: commit.config_generation,
                        commit_id: commit.commit_id.clone(),
                        created_at: now.to_string(),
                        seq: 0,
                    });
                    facets.insert(
                        key,
                        FacetHead {
                            version,
                            hash: hash.clone(),
                        },
                    );
                }
                StoredOp::Link {
                    src,
                    rel,
                    dst,
                    attrs,
                } => {
                    rows.relations.push(RelationWrite {
                        src_rid: src.clone(),
                        rel: rel.clone(),
                        dst_rid: dst.clone(),
                        attrs: match attrs {
                            Some(value) => Some(serde_json::to_string(value)?),
                            None => None,
                        },
                        commit_id: commit.commit_id.clone(),
                        seq: 0,
                    });
                }
                StoredOp::Label { rid, set, unset } => {
                    let mut head = marks.remove(rid).unwrap_or(MarkHead {
                        labels: BTreeMap::new(),
                        tombstone: false,
                        reason: None,
                    });
                    for key in unset {
                        head.labels.remove(key);
                    }
                    for (key, value) in set {
                        head.labels.insert(key.clone(), value.clone());
                    }
                    push_mark(&mut rows, rid, &head);
                    marks.insert(rid.clone(), head);
                }
                StoredOp::SetState {
                    scope, key, value, ..
                } => {
                    rows.states.push(StateWrite {
                        scope: scope.clone(),
                        key: key.clone(),
                        value: serde_json::to_string(value)?,
                        commit_id: commit.commit_id.clone(),
                        seq: 0,
                    });
                }
                StoredOp::Tombstone { rid, reason } => {
                    let mut head = marks.remove(rid).unwrap_or(MarkHead {
                        labels: BTreeMap::new(),
                        tombstone: false,
                        reason: None,
                    });
                    head.tombstone = true;
                    head.reason = Some(reason.clone());
                    push_mark(&mut rows, rid, &head);
                    marks.insert(rid.clone(), head);
                }
            }
        }
    }
    Ok(rows)
}

fn facet_key(facet: &FacetRecord) -> (String, String, String) {
    (
        facet.rid.clone(),
        facet.kind.clone(),
        facet.variant.clone().unwrap_or_default(),
    )
}

fn mark_head(mark: &MarkRecord) -> Result<MarkHead, Error> {
    let mut labels = BTreeMap::new();
    for item in &mark.labels {
        let (key, value) = item.split_once('=').ok_or(Error::InvalidLabel)?;
        labels.insert(key.to_string(), value.to_string());
    }
    Ok(MarkHead {
        labels,
        tombstone: mark.tombstone,
        reason: mark.reason.clone(),
    })
}

fn push_mark(rows: &mut DataRows, rid: &str, head: &MarkHead) {
    rows.marks.push(MarkWrite {
        rid: rid.to_string(),
        labels: head
            .labels
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect(),
        tombstone: head.tombstone,
        reason: head.reason.clone(),
        seq: 0,
    });
}

struct Spill<'a> {
    rows: &'a mut DataRows,
    seen: &'a mut HashSet<String>,
    now: &'a str,
    inline_text_max: u64,
}

async fn materialize(
    staging: &Staging,
    body: &StoredBody,
    hash: &str,
    media_type: &Option<String>,
    spill: &mut Spill<'_>,
) -> Result<(Option<String>, Option<String>, Option<String>), Error> {
    match body {
        StoredBody::Text { text } => {
            let bytes = text.as_bytes();
            if content_hash(bytes) != hash {
                return Err(Error::BodyMismatch);
            }
            if bytes.len() as u64 > spill.inline_text_max {
                push_blob(
                    spill.rows,
                    spill.seen,
                    hash,
                    bytes,
                    media_type.clone(),
                    spill.now,
                );
                Ok((None, None, Some(hash.to_string())))
            } else {
                Ok((Some(text.clone()), None, None))
            }
        }
        StoredBody::Struct { value } => {
            let bytes = serde_json::to_vec(value)?;
            if content_hash(&bytes) != hash {
                return Err(Error::BodyMismatch);
            }
            let text = String::from_utf8(bytes).map_err(|_| Error::BodyMismatch)?;
            Ok((None, Some(text), None))
        }
        StoredBody::Staged { sha256, size } => {
            let bytes = staging.read(sha256).await?;
            if bytes.len() as u64 != *size
                || content_hash(&bytes) != *sha256
                || sha256.as_str() != hash
            {
                return Err(Error::StagingMismatch(sha256.clone()));
            }
            push_blob(
                spill.rows,
                spill.seen,
                sha256,
                &bytes,
                media_type.clone(),
                spill.now,
            );
            Ok((None, None, Some(sha256.clone())))
        }
    }
}

fn push_blob(
    rows: &mut DataRows,
    seen: &mut HashSet<String>,
    sha: &str,
    bytes: &[u8],
    media_type: Option<String>,
    now: &str,
) {
    if !seen.insert(sha.to_string()) {
        return;
    }
    rows.blobs.push(BlobWrite {
        sha256: sha.to_string(),
        size: bytes.len() as u64,
        media_type,
        bytes: bytes.to_vec(),
        created_at: now.to_string(),
        seq: 0,
    });
}

#[cfg(test)]
fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) -> Result<(), Error> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk(&path, root, out)?;
        } else {
            let relative = path.strip_prefix(root).unwrap_or(&path);
            out.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}
