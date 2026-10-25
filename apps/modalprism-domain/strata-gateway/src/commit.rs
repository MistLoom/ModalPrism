//! 变更集落地：解析别名、物化正文、写入行、发布快照与恢复。
//!
//! 接受阶段只写版本化行，发布前不可见；发布阶段钉住各表版本，
//! 把提交标为可见并触发派生。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use strata_model::{
    Body, ChangeSet, CommitId, CommitResult, FacetKind, IdentityKey, Namespace, Op, Rid, Target,
    Timestamp, WaitFor, content_hash,
};
use ulid::Ulid;

use crate::blob::{BlobService, PreparedBlob};
use crate::config::Options;
use crate::data::{
    self, Batch, BlobRow, CommitRow, Dataset, FacetRow, IdentityRow, MarkRow, RelationRow,
    ResourceRow, SnapshotRow, StateRow,
};
use crate::db::Db;
use crate::derive;
use crate::error::{Error, Result};
use crate::staging::Staging;
use crate::task;

/// 已写入的数据头，供本次接续使用。只包含行为上需要的字段。
#[derive(Debug, Default)]
struct Heads {
    identities: HashMap<String, String>,
    resources: HashSet<String>,
    states: HashMap<(String, String), u64>,
    marks: HashMap<String, MarkHead>,
}

#[derive(Clone, Debug, Default)]
struct MarkHead {
    labels: BTreeMap<String, String>,
    tombstone: bool,
    reason: Option<String>,
}

struct Planned {
    batch: Batch,
    staging: Vec<String>,
    ack: Option<String>,
}

/// 接收一个变更集。返回 `Accepted` 结果，行在发布前不可见。
pub async fn accept(
    db: &Db,
    blobs: &BlobService,
    staging: &Staging,
    options: &Options,
    changeset: &ChangeSet,
    now: Timestamp,
) -> Result<CommitResult> {
    changeset.validate()?;
    for op in &changeset.ops {
        if let Op::PutFacet {
            body: Body::Staged { part },
            ..
        } = op
        {
            staging.check(part).await?;
        }
    }
    if let Some(key) = &changeset.idempotency_key {
        if let Some(found) = data::find_commit_by_key(db, key).await? {
            return outcome(&found, true);
        }
    }
    let commit_id = CommitId::new(Ulid::new().to_string())?;
    let now_text = now.to_rfc3339()?;
    let mut heads = load_heads(db).await?;
    let planned = resolve(
        changeset,
        &mut heads,
        commit_id.as_str(),
        &now_text,
        staging,
        blobs,
        options.inline_text_max,
    )
    .await?;
    let mut batch = planned.batch;
    let mut tx = db.begin().await?;
    data::write_in(&mut tx, &mut batch).await?;
    data::insert_commit(
        &mut tx,
        &CommitRow {
            commit_id: commit_id.as_str().to_string(),
            idempotency_key: changeset.idempotency_key.clone(),
            producer_module: changeset.producer.module.clone(),
            producer_version: changeset.producer.version.clone(),
            config_generation: changeset.producer.config_generation,
            snapshot_seq: None,
            status: "accepted".into(),
            staging: planned.staging,
            created_at: now_text,
        },
    )
    .await?;
    if let Some(task_id) = planned.ack {
        task::ack_in(&mut tx, &strata_model::TaskId::new(task_id)?).await?;
    }
    tx.commit().await?;
    Ok(CommitResult {
        commit_id,
        status: WaitFor::Accepted,
        snapshot: None,
        duplicate: false,
    })
}

fn outcome(row: &CommitRow, duplicate: bool) -> Result<CommitResult> {
    Ok(CommitResult {
        commit_id: CommitId::new(row.commit_id.clone())?,
        status: if row.snapshot_seq.is_some() {
            WaitFor::Visible
        } else {
            WaitFor::Accepted
        },
        snapshot: row.snapshot_seq,
        duplicate,
    })
}

async fn load_heads(db: &Db) -> Result<Heads> {
    let mut marks = HashMap::new();
    for (rid, row) in data::mark_heads(db).await? {
        marks.insert(rid, mark_head(&row)?);
    }
    Ok(Heads {
        identities: data::identity_map(db).await?,
        resources: data::resource_rids(db).await?,
        states: data::state_versions(db).await?,
        marks,
    })
}

fn mark_head(row: &MarkRow) -> Result<MarkHead> {
    let mut labels = BTreeMap::new();
    for item in &row.labels {
        let (key, value) = item.split_once('=').ok_or(Error::InvalidLabel)?;
        labels.insert(key.to_string(), value.to_string());
    }
    Ok(MarkHead {
        labels,
        tombstone: row.tombstone,
        reason: row.reason.clone(),
    })
}

async fn resolve(
    changeset: &ChangeSet,
    heads: &mut Heads,
    commit_id: &str,
    now: &str,
    staging: &Staging,
    blobs: &BlobService,
    inline_text_max: u64,
) -> Result<Planned> {
    let mut aliases = HashMap::<String, String>::new();
    let mut batch = Batch::default();
    let mut staged = Vec::new();
    let mut blobs_seen = HashSet::new();
    let mut spill = Spill {
        rows: Vec::new(),
        blobs,
        seen: &mut blobs_seen,
        now,
        inline_text_max,
    };

    for op in &changeset.ops {
        match op {
            Op::Ensure {
                identity,
                ns,
                labels,
                r#as,
            } => {
                check_labels(labels)?;
                if aliases.contains_key(r#as) {
                    return Err(Error::DuplicateAlias(r#as.clone()));
                }
                let ns = match ns {
                    Some(ns) => Namespace::parse(ns)?.to_string(),
                    None => Namespace::Data.to_string(),
                };
                let key = IdentityKey::parse(identity)?;
                let source_id = match &key {
                    IdentityKey::Source { source_id, .. } => Some(source_id.clone()),
                    _ => None,
                };
                if let Some(existing) = heads.identities.get(identity) {
                    aliases.insert(r#as.clone(), existing.clone());
                    continue;
                }
                let rid = Rid::generate().to_string();
                aliases.insert(r#as.clone(), rid.clone());
                heads.identities.insert(identity.clone(), rid.clone());
                heads.resources.insert(rid.clone());
                batch.identities.push(IdentityRow {
                    identity_key: identity.clone(),
                    rid: rid.clone(),
                    scheme: scheme_of(identity).to_string(),
                    commit_id: commit_id.to_string(),
                    seq: 0,
                    v: 0,
                });
                batch.resources.push(ResourceRow {
                    rid: rid.clone(),
                    ns,
                    source_id,
                    created_at: now.to_string(),
                    seq: 0,
                    v: 0,
                });
                if !labels.is_empty() {
                    let head = MarkHead {
                        labels: labels.clone(),
                        tombstone: false,
                        reason: None,
                    };
                    push_mark(&mut batch, &rid, &head);
                    heads.marks.insert(rid, head);
                }
            }
            Op::PutFacet {
                target,
                kind,
                variant,
                body,
                media_type,
                lineage,
            } => {
                if matches!(kind, FacetKind::Embedding) {
                    return Err(Error::EmbeddingUnspecified);
                }
                let rid = resolve_target(target, &aliases, heads)?;
                let stored = store_body(kind, body)?;
                if let StoredBody::Staged { sha256, .. } = &stored {
                    staged.push(sha256.clone());
                }
                let (hash, size) = body_hash(&stored)?;
                let (body_text, body_struct, blob_ref) =
                    materialize(staging, &stored, &hash, media_type.clone(), &mut spill).await?;
                batch.facets.push(FacetRow {
                    rid,
                    kind: kind.as_str().to_string(),
                    variant: variant.clone(),
                    version: 0,
                    content_hash: hash,
                    media_type: media_type.clone(),
                    size,
                    body_text,
                    body_struct,
                    blob_ref,
                    inputs: lineage.inputs.iter().map(|span| span.to_string()).collect(),
                    params_hash: lineage.params_hash.clone(),
                    producer_module: changeset.producer.module.clone(),
                    producer_version: changeset.producer.version.clone(),
                    config_generation: changeset.producer.config_generation,
                    commit_id: commit_id.to_string(),
                    created_at: now.to_string(),
                    seq: 0,
                    v: 0,
                });
            }
            Op::Link {
                src,
                rel,
                dst,
                attrs,
            } => {
                batch.relations.push(RelationRow {
                    src_rid: resolve_target(src, &aliases, heads)?,
                    rel: rel.clone(),
                    dst_rid: resolve_target(dst, &aliases, heads)?,
                    attrs: attrs.clone(),
                    commit_id: commit_id.to_string(),
                    seq: 0,
                    v: 0,
                });
            }
            Op::Label { target, set, unset } => {
                check_labels(set)?;
                if unset.iter().any(|key| bad_label(key)) {
                    return Err(Error::InvalidLabel);
                }
                let rid = resolve_target(target, &aliases, heads)?;
                let mut head = heads.marks.remove(&rid).unwrap_or_default();
                for key in unset {
                    head.labels.remove(key);
                }
                for (key, value) in set {
                    head.labels.insert(key.clone(), value.clone());
                }
                push_mark(&mut batch, &rid, &head);
                heads.marks.insert(rid, head);
            }
            Op::SetState {
                scope,
                key,
                value,
                expect_version,
            } => {
                let actual = heads
                    .states
                    .get(&(scope.clone(), key.clone()))
                    .copied()
                    .unwrap_or(0);
                if let Some(expected) = expect_version {
                    if *expected != actual {
                        return Err(Error::StateVersion {
                            scope: scope.clone(),
                            key: key.clone(),
                            expected: *expected,
                            actual,
                        });
                    }
                }
                let version = actual + 1;
                heads.states.insert((scope.clone(), key.clone()), version);
                batch.states.push(StateRow {
                    scope: scope.clone(),
                    key: key.clone(),
                    value: value.clone(),
                    commit_id: commit_id.to_string(),
                    seq: 0,
                    v: 0,
                });
            }
            Op::Tombstone { target, reason } => {
                let rid = resolve_target(target, &aliases, heads)?;
                let mut head = heads.marks.remove(&rid).unwrap_or_default();
                head.tombstone = true;
                head.reason = Some(reason.clone());
                push_mark(&mut batch, &rid, &head);
                heads.marks.insert(rid, head);
            }
            Op::Restore { target, reason } => {
                let rid = resolve_target(target, &aliases, heads)?;
                let mut head = heads.marks.remove(&rid).unwrap_or_default();
                head.tombstone = false;
                head.reason = Some(reason.clone());
                push_mark(&mut batch, &rid, &head);
                heads.marks.insert(rid, head);
            }
        }
    }

    batch.blobs.extend(spill.rows);
    Ok(Planned {
        batch,
        staging: staged,
        ack: changeset.ack.as_ref().map(|id| id.as_str().to_string()),
    })
}

fn resolve_target(
    target: &Target,
    aliases: &HashMap<String, String>,
    heads: &Heads,
) -> Result<String> {
    match target {
        Target::Alias(alias) => aliases
            .get(alias)
            .cloned()
            .ok_or_else(|| Error::UnknownAlias(alias.clone())),
        Target::Rid(rid) => {
            let text = rid.as_str();
            if heads.resources.contains(text) {
                Ok(text.to_string())
            } else {
                Err(Error::UnknownResource(text.to_string()))
            }
        }
    }
}

enum StoredBody {
    Text { text: String },
    Struct { value: serde_json::Value },
    Staged { sha256: String, size: u64 },
}

fn store_body(kind: &FacetKind, body: &Body) -> Result<StoredBody> {
    match body {
        Body::External { .. } => Err(Error::ExternalUnspecified),
        Body::Text { text } if text_kind(kind) => Ok(StoredBody::Text { text: text.clone() }),
        Body::Struct { value } if struct_kind(kind) => Ok(StoredBody::Struct {
            value: value.clone(),
        }),
        Body::Staged { part } if binary_kind(kind) => Ok(StoredBody::Staged {
            sha256: part.sha256().to_string(),
            size: part.size(),
        }),
        _ => Err(Error::BodyMismatch),
    }
}

fn body_hash(body: &StoredBody) -> Result<(String, u64)> {
    match body {
        StoredBody::Text { text } => Ok((content_hash(text.as_bytes()), text.len() as u64)),
        StoredBody::Struct { value } => {
            let bytes = serde_json::to_vec(value)?;
            let size = bytes.len() as u64;
            Ok((content_hash(&bytes), size))
        }
        StoredBody::Staged { sha256, size } => Ok((sha256.clone(), *size)),
    }
}

fn text_kind(kind: &FacetKind) -> bool {
    matches!(
        kind,
        FacetKind::Raw
            | FacetKind::Html
            | FacetKind::Xml
            | FacetKind::Md
            | FacetKind::Text
            | FacetKind::Json
            | FacetKind::Yaml
            | FacetKind::Other(_)
    )
}

fn struct_kind(kind: &FacetKind) -> bool {
    matches!(
        kind,
        FacetKind::Struct
            | FacetKind::Links
            | FacetKind::Regions
            | FacetKind::Json
            | FacetKind::Yaml
            | FacetKind::Other(_)
    )
}

fn binary_kind(kind: &FacetKind) -> bool {
    matches!(
        kind,
        FacetKind::Raw
            | FacetKind::Html
            | FacetKind::Xml
            | FacetKind::Md
            | FacetKind::Text
            | FacetKind::Json
            | FacetKind::Yaml
            | FacetKind::Image
            | FacetKind::Audio
            | FacetKind::Video
            | FacetKind::File
            | FacetKind::Other(_)
    )
}

fn scheme_of(identity: &str) -> &str {
    identity.split(':').next().unwrap_or("")
}

fn bad_label(text: &str) -> bool {
    text.contains('=') || text.contains('\n')
}

fn check_labels(labels: &BTreeMap<String, String>) -> Result<()> {
    if labels.keys().any(|key| bad_label(key)) || labels.values().any(|value| bad_label(value)) {
        Err(Error::InvalidLabel)
    } else {
        Ok(())
    }
}

fn push_mark(batch: &mut Batch, rid: &str, head: &MarkHead) {
    batch.marks.push(MarkRow {
        rid: rid.to_string(),
        labels: head
            .labels
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect(),
        tombstone: head.tombstone,
        reason: head.reason.clone(),
        seq: 0,
        v: 0,
    });
}

struct Spill<'a> {
    rows: Vec<BlobRow>,
    blobs: &'a BlobService,
    seen: &'a mut HashSet<String>,
    now: &'a str,
    inline_text_max: u64,
}

async fn materialize(
    staging: &Staging,
    body: &StoredBody,
    hash: &str,
    media_type: Option<String>,
    spill: &mut Spill<'_>,
) -> Result<(Option<String>, Option<String>, Option<String>)> {
    match body {
        StoredBody::Text { text } => {
            let bytes = text.as_bytes();
            if content_hash(bytes) != hash {
                return Err(Error::BodyMismatch);
            }
            if bytes.len() as u64 > spill.inline_text_max {
                let prepared = spill.blobs.prepare(bytes).await?;
                spill.push(prepared, media_type, spill.now);
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
            let prepared = spill.blobs.prepare(&bytes).await?;
            spill.push(prepared, media_type, spill.now);
            Ok((None, None, Some(sha256.clone())))
        }
    }
}

impl Spill<'_> {
    fn push(&mut self, prepared: PreparedBlob, media_type: Option<String>, now: &str) {
        if !self.seen.insert(prepared.sha256.clone()) {
            return;
        }
        self.rows.push(BlobRow {
            sha256: prepared.sha256,
            size: prepared.size,
            media_type,
            tier: prepared.tier,
            inline: prepared.inline,
            created_at: now.to_string(),
            seq: 0,
            v: 0,
        });
    }
}

/// 发布所有已接收提交，返回快照序号；没有待发布提交时返回 `None`。
pub async fn flush(
    db: &Db,
    staging: &Staging,
    options: &Options,
    trigger: impl AsyncFnOnce(&SnapshotRow) -> Result<()>,
) -> Result<Option<u64>> {
    recover(db, staging, options.staging_ttl).await?;
    let pending = data::pending_commits(db).await?;
    if pending.is_empty() {
        let keep = data::accepted_staging(db).await?;
        staging.sweep(&keep, options.staging_ttl).await?;
        return Ok(None);
    }
    let ids: Vec<String> = pending.iter().map(|row| row.commit_id.clone()).collect();
    let generation = pending
        .iter()
        .map(|row| row.config_generation)
        .max()
        .unwrap_or(0);
    let published_at = Timestamp::now().to_rfc3339()?;
    let snapshot = data::write_snapshot(db, &ids, "data", generation, &published_at).await?;
    data::mark_visible(db, &ids, snapshot.seq).await?;
    trigger(&snapshot).await?;
    let keep = data::accepted_staging(db).await?;
    for sha in pending.iter().flat_map(|row| row.staging.clone()) {
        if !keep.contains(&sha) {
            staging.remove(&sha).await?;
        }
    }
    staging.sweep(&keep, options.staging_ttl).await?;
    Ok(Some(snapshot.seq))
}

/// 重启后把已进入快照的提交补成可见，并清理暂存区。
pub async fn recover(db: &Db, staging: &Staging, ttl: Duration) -> Result<()> {
    for snapshot in data::snapshots(db).await? {
        data::mark_visible(db, &snapshot.commit_ids, snapshot.seq).await?;
    }
    let keep = data::accepted_staging(db).await?;
    staging.sweep(&keep, ttl).await?;
    Ok(())
}

/// 最新快照的可读视图。
pub async fn view_latest(db: &Db) -> Result<Dataset> {
    match data::latest_snapshot(db).await? {
        Some(snapshot) => dataset_of(db, snapshot).await,
        None => Ok(Dataset::default()),
    }
}

pub async fn view_snapshot(db: &Db, seq: u64) -> Result<Dataset> {
    let snapshot = data::snapshots(db)
        .await?
        .into_iter()
        .find(|row| row.seq == seq)
        .ok_or(Error::MissingSnapshot(seq))?;
    dataset_of(db, snapshot).await
}

pub async fn view_time(db: &Db, time: Timestamp) -> Result<Dataset> {
    let mut best: Option<SnapshotRow> = None;
    for row in data::snapshots(db).await? {
        let published = Timestamp::parse(&row.published_at)?;
        if published <= time && best.as_ref().is_none_or(|current| row.seq > current.seq) {
            best = Some(row);
        }
    }
    match best {
        Some(snapshot) => dataset_of(db, snapshot).await,
        None => Ok(Dataset::default()),
    }
}

async fn dataset_of(db: &Db, snapshot: SnapshotRow) -> Result<Dataset> {
    let mut dataset = data::read_at(db, &snapshot.versions).await?;
    dataset.snapshot = Some(snapshot);
    Ok(dataset)
}

/// 按快照顺序列出可见变更。
pub async fn changes(db: &Db, after: u64, limit: Option<u32>) -> Result<Vec<SnapshotRow>> {
    let mut rows: Vec<_> = data::snapshots(db)
        .await?
        .into_iter()
        .filter(|row| row.seq > after)
        .collect();
    rows.sort_by_key(|row| row.seq);
    if let Some(limit) = limit {
        rows.truncate(limit as usize);
    }
    Ok(rows)
}

/// 触发派生。读的是刚发布的快照，任务按声明去重入队。
pub async fn trigger(db: &Db, options: &Options, snapshot: &SnapshotRow) -> Result<()> {
    if options.derivations.is_empty() {
        return Ok(());
    }
    let dataset = data::read_at(db, &snapshot.versions).await?;
    derive::trigger(db, &options.derivations, &dataset, snapshot.seq).await?;
    Ok(())
}
