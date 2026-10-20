//! 进程内的 Strata 入口。调用方不接触存储引擎的表 API。

mod query;

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use bytes::Bytes;
use serde_json::Value;
use strata_model::{
    AccessTask, ApplyOpts, AsOf, ChangeSet, CommitId, CommitResult, EdgeDir, FacetKind, FacetRef,
    FacetSummary, LineageDir, PartRef, Query, QueueName, Rid, SpanRef, TaskId, Timestamp,
    VersionSel,
};
use strata_store::{Engine, FacetRecord, View};

pub use strata_store::Options;

pub struct Strata {
    engine: Engine,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Facet {
    pub version: u64,
    pub content_hash: String,
    pub media_type: Option<String>,
    pub size: u64,
    pub text: Option<String>,
    pub value: Option<Value>,
    pub blob: Option<PartRef>,
    pub bytes: Option<Bytes>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FacetVersion {
    pub version: u64,
    pub content_hash: String,
    pub created_at: Timestamp,
    pub seq: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Relation {
    pub src: Rid,
    pub rel: String,
    pub dst: Rid,
    pub attrs: Option<Value>,
    pub commit_id: CommitId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineageNode {
    pub facet: FacetRef,
    pub module: String,
    pub version: String,
    pub params_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineageEdge {
    pub from: FacetRef,
    pub to: FacetRef,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineageGraph {
    pub nodes: Vec<LineageNode>,
    pub edges: Vec<LineageEdge>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisibleChange {
    pub snapshot: u64,
    pub commit_ids: Vec<CommitId>,
    pub published_at: Timestamp,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StateValue {
    pub value: Value,
    pub version: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QueryPage {
    pub rows: Vec<Value>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Store(#[from] strata_store::Error),
    #[error("不支持的查询表达式")]
    UnsupportedExpr,
    #[error("检索未定义")]
    SearchUnspecified,
    #[error("标签未定义")]
    TagUnspecified,
    #[error("facet 不存在")]
    MissingFacet,
    #[error("区间超出正文")]
    SpanOutOfRange,
    #[error("正文不是字节")]
    NotBytes,
    #[error("未知字段: {0}")]
    UnknownField(String),
    #[error("排序与 cursor 不能同时使用")]
    CursorConflict,
    #[error("查询形状无效")]
    QueryShape,
    #[error("cursor 无效")]
    InvalidCursor,
}

impl From<strata_model::ModelError> for Error {
    fn from(value: strata_model::ModelError) -> Self {
        Self::Store(strata_store::Error::from(value))
    }
}

impl Strata {
    pub async fn open(root: impl AsRef<std::path::Path>, options: Options) -> Result<Self, Error> {
        Ok(Self {
            engine: Engine::open(root, options).await?,
        })
    }

    pub async fn stage(&self, bytes: &[u8]) -> Result<PartRef, Error> {
        Ok(self.engine.stage(bytes).await?)
    }

    pub async fn apply(
        &self,
        changeset: ChangeSet,
        opts: ApplyOpts,
    ) -> Result<CommitResult, Error> {
        Ok(self.engine.apply(changeset, opts).await?)
    }

    pub async fn flush(&self) -> Result<Option<u64>, Error> {
        Ok(self.engine.flush().await?)
    }

    pub async fn recover(&self) -> Result<(), Error> {
        Ok(self.engine.recover().await?)
    }

    pub async fn kinds(
        &self,
        rid: &Rid,
        include_variants: bool,
    ) -> Result<Vec<FacetSummary>, Error> {
        let view = self.engine.view_latest().await?;
        let mut groups: HashMap<(String, Option<String>), Vec<&FacetRecord>> = HashMap::new();
        for facet in view.facets.iter().filter(|facet| facet.rid == rid.as_str()) {
            let variant = if include_variants {
                facet.variant.clone()
            } else {
                None
            };
            groups
                .entry((facet.kind.clone(), variant))
                .or_default()
                .push(facet);
        }
        let mut out = Vec::new();
        for ((kind, variant), facets) in groups {
            let latest = facets
                .iter()
                .max_by_key(|facet| facet.version)
                .ok_or(Error::MissingFacet)?;
            out.push(FacetSummary {
                kind: FacetKind::parse(&kind)?,
                variant,
                latest: latest.version,
                versions: u32::try_from(facets.len()).unwrap_or(u32::MAX),
                media_type: latest.media_type.clone(),
                size: Some(latest.size),
                updated_at: Timestamp::parse(&latest.created_at)?,
            });
        }
        out.sort_by(|left, right| {
            left.kind
                .as_str()
                .cmp(right.kind.as_str())
                .then(left.variant.cmp(&right.variant))
        });
        Ok(out)
    }

    pub async fn has(
        &self,
        rid: &Rid,
        kind: &FacetKind,
        variant: Option<&str>,
    ) -> Result<bool, Error> {
        let view = self.engine.view_latest().await?;
        Ok(view.facets.iter().any(|facet| {
            facet.rid == rid.as_str()
                && facet.kind == kind.as_str()
                && variant.is_none_or(|expected| facet.variant.as_deref() == Some(expected))
        }))
    }

    pub async fn get(&self, reference: &FacetRef, lazy: bool) -> Result<Option<Facet>, Error> {
        let view = self.engine.view_latest().await?;
        let Some(facet) = find_facet(&view, reference) else {
            return Ok(None);
        };
        Ok(Some(load_facet(&self.engine, &view, facet, lazy).await?))
    }

    pub async fn slice(&self, span: &SpanRef) -> Result<Bytes, Error> {
        let view = self.engine.view_latest().await?;
        let facet = find_facet(&view, &span.facet).ok_or(Error::MissingFacet)?;
        if facet.body_struct.is_some() && facet.body_text.is_none() && facet.blob_ref.is_none() {
            return Err(Error::NotBytes);
        }
        if span.end > facet.size {
            return Err(Error::SpanOutOfRange);
        }
        let start = usize::try_from(span.start).map_err(|_| Error::SpanOutOfRange)?;
        let end = usize::try_from(span.end).map_err(|_| Error::SpanOutOfRange)?;
        if let Some(text) = &facet.body_text {
            return Ok(Bytes::copy_from_slice(&text.as_bytes()[start..end]));
        }
        let Some(sha) = facet.blob_ref.clone() else {
            return Err(Error::NotBytes);
        };
        let versions = view
            .snapshot
            .as_ref()
            .ok_or(Error::MissingFacet)?
            .versions
            .clone();
        let bytes = self
            .engine
            .read_blob_range(&versions, &sha, span.start, span.end - span.start)
            .await?;
        Ok(Bytes::from(bytes))
    }

    pub async fn resolve(&self, identity_key: &str) -> Result<Option<Rid>, Error> {
        let view = self.engine.view_latest().await?;
        let Some(identity) = view
            .identities
            .iter()
            .find(|item| item.identity_key == identity_key)
        else {
            return Ok(None);
        };
        Ok(Some(Rid::parse(&identity.rid)?))
    }

    pub async fn history(&self, reference: &FacetRef) -> Result<Vec<FacetVersion>, Error> {
        let view = self.engine.view_latest().await?;
        let mut versions: Vec<_> = view
            .facets
            .iter()
            .filter(|facet| same_facet(facet, reference, false))
            .map(|facet| {
                Ok(FacetVersion {
                    version: facet.version,
                    content_hash: facet.content_hash.clone(),
                    created_at: Timestamp::parse(&facet.created_at)?,
                    seq: facet.seq,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        versions.sort_by_key(|item| item.version);
        Ok(versions)
    }

    pub async fn lineage(
        &self,
        reference: &FacetRef,
        dir: LineageDir,
        depth: u32,
    ) -> Result<LineageGraph, Error> {
        let view = self.engine.view_latest().await?;
        let start = find_facet(&view, reference).ok_or(Error::MissingFacet)?;
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let mut seen = HashSet::new();
        walk_lineage(&view, start, dir, depth, &mut nodes, &mut edges, &mut seen)?;
        Ok(LineageGraph { nodes, edges })
    }

    pub async fn related(
        &self,
        rid: &Rid,
        rel: Option<&str>,
        dir: EdgeDir,
    ) -> Result<Vec<Relation>, Error> {
        let view = self.engine.view_latest().await?;
        let mut out = Vec::new();
        for relation in &view.relations {
            let hit = match dir {
                EdgeDir::Out => relation.src_rid == rid.as_str(),
                EdgeDir::In => relation.dst_rid == rid.as_str(),
            };
            if !hit || rel.is_some_and(|expected| relation.rel != expected) {
                continue;
            }
            out.push(Relation {
                src: Rid::parse(&relation.src_rid)?,
                rel: relation.rel.clone(),
                dst: Rid::parse(&relation.dst_rid)?,
                attrs: relation.attrs.clone(),
                commit_id: CommitId::new(relation.commit_id.clone())?,
            });
        }
        Ok(out)
    }

    pub async fn query(&self, query: Query) -> Result<QueryPage, Error> {
        query.validate()?;
        if query.search.is_some() {
            return Err(Error::SearchUnspecified);
        }
        let view = match &query.as_of {
            Some(AsOf::Tag(_)) => return Err(Error::TagUnspecified),
            Some(AsOf::Snapshot(seq)) => self.engine.view_snapshot(*seq).await?,
            Some(AsOf::Time(time)) => self.engine.view_time(*time).await?,
            None => self.engine.view_latest().await?,
        };
        let page = query::execute(&view, &query)?;
        Ok(QueryPage {
            rows: page.rows,
            next_cursor: page.next_cursor,
        })
    }

    pub async fn changes(
        &self,
        after: u64,
        limit: Option<u32>,
    ) -> Result<Vec<VisibleChange>, Error> {
        let rows = self.engine.changes(after, limit).await?;
        rows.into_iter()
            .map(|row| {
                Ok(VisibleChange {
                    snapshot: row.seq,
                    commit_ids: row
                        .commit_ids
                        .into_iter()
                        .map(CommitId::new)
                        .collect::<Result<Vec<_>, _>>()?,
                    published_at: Timestamp::parse(&row.published_at)?,
                })
            })
            .collect()
    }

    pub async fn state(&self, scope: &str, key: &str) -> Result<Option<StateValue>, Error> {
        let view = self.engine.view_latest().await?;
        let rows: Vec<_> = view
            .states
            .iter()
            .filter(|row| row.scope == scope && row.key == key)
            .collect();
        let Some(latest) = rows.iter().max_by_key(|row| row.seq) else {
            return Ok(None);
        };
        Ok(Some(StateValue {
            value: latest.value.clone(),
            version: u64::try_from(rows.len()).unwrap_or(u64::MAX),
        }))
    }

    pub async fn enqueue(&self, tasks: &[AccessTask]) -> Result<(), Error> {
        Ok(self.engine.enqueue(tasks).await?)
    }

    pub async fn lease(
        &self,
        queue: QueueName,
        count: usize,
        lease_for: Duration,
    ) -> Result<Vec<AccessTask>, Error> {
        Ok(self.engine.lease(queue, count, lease_for).await?)
    }

    pub async fn extend(&self, task_id: &TaskId, lease_for: Duration) -> Result<(), Error> {
        Ok(self.engine.extend(task_id, lease_for).await?)
    }

    pub async fn nack(
        &self,
        task_id: &TaskId,
        retry_at: Timestamp,
        error: &str,
    ) -> Result<(), Error> {
        Ok(self.engine.nack(task_id, retry_at, error).await?)
    }

    pub async fn lease_id(
        &self,
        task_id: &TaskId,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<AccessTask, Error> {
        Ok(self.engine.lease_id(task_id, lease_for, now).await?)
    }

    pub async fn release_leases(&self) -> Result<Vec<AccessTask>, Error> {
        Ok(self.engine.release_leases().await?)
    }

    pub async fn leased_count(&self, queue: QueueName, now: Timestamp) -> Result<usize, Error> {
        Ok(self.engine.leased_count(queue, now).await?)
    }

    pub async fn task_active(&self, source: &str, locator_key: &str) -> Result<bool, Error> {
        Ok(self.engine.task_active(source, locator_key).await?)
    }

    pub async fn lease_ready(
        &self,
        source: &str,
        locator_key: &str,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<Option<AccessTask>, Error> {
        Ok(self
            .engine
            .lease_ready(source, locator_key, lease_for, now)
            .await?)
    }

    pub async fn accepted_state(
        &self,
        scope: &str,
        key: &str,
    ) -> Result<Option<StateValue>, Error> {
        let Some((version, value)) = self.engine.accepted_state(scope, key).await? else {
            return Ok(None);
        };
        Ok(Some(StateValue {
            value: serde_json::from_str(&value).map_err(strata_store::Error::from)?,
            version,
        }))
    }

    pub async fn hot_get(&self, kind: &str, key: &str) -> Result<Option<Value>, Error> {
        let Some(value) = self.engine.hot_get(kind, key).await? else {
            return Ok(None);
        };
        Ok(Some(
            serde_json::from_str(&value).map_err(strata_store::Error::from)?,
        ))
    }

    pub async fn hot_put(&self, kind: &str, key: &str, value: &Value) -> Result<(), Error> {
        let text = serde_json::to_string(value).map_err(strata_store::Error::from)?;
        Ok(self.engine.hot_put(kind, key, &text).await?)
    }
}

fn same_facet(facet: &FacetRecord, reference: &FacetRef, exact_version: bool) -> bool {
    if facet.rid != reference.rid.as_str() || facet.kind != reference.kind.as_str() {
        return false;
    }
    if facet.variant.as_deref() != reference.variant.as_deref() {
        return false;
    }
    if exact_version {
        match reference.version {
            VersionSel::Latest => true,
            VersionSel::Exact(version) => facet.version == version,
        }
    } else {
        true
    }
}

fn find_facet<'a>(view: &'a View, reference: &FacetRef) -> Option<&'a FacetRecord> {
    view.facets
        .iter()
        .filter(|facet| same_facet(facet, reference, true))
        .max_by_key(|facet| facet.version)
}

async fn load_facet(
    engine: &Engine,
    view: &View,
    facet: &FacetRecord,
    lazy: bool,
) -> Result<Facet, Error> {
    let value = match &facet.body_struct {
        Some(raw) => Some(serde_json::from_str(raw).map_err(strata_store::Error::from)?),
        None => None,
    };
    let mut text = facet.body_text.clone();
    let blob = match &facet.blob_ref {
        Some(sha) => Some(PartRef::new(sha.clone(), facet.size)?),
        None => None,
    };
    let bytes = if lazy || facet.blob_ref.is_none() || text.is_some() {
        None
    } else {
        let sha = facet.blob_ref.as_deref().ok_or(Error::MissingFacet)?;
        let versions = view.snapshot.clone().ok_or(Error::MissingFacet)?.versions;
        Some(Bytes::from(engine.read_blob(&versions, sha).await?))
    };
    if lazy && facet.body_text.is_none() {
        text = None;
    }
    Ok(Facet {
        version: facet.version,
        content_hash: facet.content_hash.clone(),
        media_type: facet.media_type.clone(),
        size: facet.size,
        text,
        value,
        blob,
        bytes,
    })
}

fn walk_lineage(
    view: &View,
    facet: &FacetRecord,
    dir: LineageDir,
    depth: u32,
    nodes: &mut Vec<LineageNode>,
    edges: &mut Vec<LineageEdge>,
    seen: &mut HashSet<(String, String, String, u64)>,
) -> Result<(), Error> {
    let key = (
        facet.rid.clone(),
        facet.kind.clone(),
        facet.variant.clone().unwrap_or_default(),
        facet.version,
    );
    if !seen.insert(key) {
        return Ok(());
    }
    let current = facet_ref(facet)?;
    nodes.push(LineageNode {
        facet: current.clone(),
        module: facet.producer_module.clone(),
        version: facet.producer_version.clone(),
        params_hash: facet.params_hash.clone(),
    });
    if depth == 0 {
        return Ok(());
    }
    match dir {
        LineageDir::Up => {
            for input in &facet.inputs {
                let span = SpanRef::parse(input)?;
                let Some(next) = find_facet(view, &span.facet) else {
                    return Err(Error::MissingFacet);
                };
                edges.push(LineageEdge {
                    from: current.clone(),
                    to: facet_ref(next)?,
                });
                walk_lineage(view, next, dir, depth - 1, nodes, edges, seen)?;
            }
        }
        LineageDir::Down => {
            for candidate in &view.facets {
                for input in &candidate.inputs {
                    let span = SpanRef::parse(input)?;
                    if span.facet.rid != current.rid
                        || span.facet.kind != current.kind
                        || span.facet.variant != current.variant
                    {
                        continue;
                    }
                    let version_hit = match span.facet.version {
                        VersionSel::Exact(version) => version == facet.version,
                        VersionSel::Latest => true,
                    };
                    if !version_hit {
                        continue;
                    }
                    edges.push(LineageEdge {
                        from: current.clone(),
                        to: facet_ref(candidate)?,
                    });
                    walk_lineage(view, candidate, dir, depth - 1, nodes, edges, seen)?;
                }
            }
        }
    }
    Ok(())
}

fn facet_ref(facet: &FacetRecord) -> Result<FacetRef, Error> {
    Ok(FacetRef::new(
        Rid::parse(&facet.rid)?,
        FacetKind::parse(&facet.kind)?,
        facet.variant.clone(),
        VersionSel::Exact(facet.version),
    )?)
}
