//! 进程内的 Strata 入口。调用方不接触数据层的表 API。

mod derive;
mod options;
mod query;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use bytes::Bytes;
use serde_json::Value;
use strata_gateway::{
    Dataset as View, FacetRow as FacetRecord, Gateway, Request as GatewayRequest, StateRow,
};
use strata_model::{
    AccessTask, ApplyOpts, AsOf, ChangeSet, CommitId, CommitResult, EdgeDir, FacetKind, FacetRef,
    FacetSummary, LineageDir, PartRef, Query, QueueName, Rid, SpanRef, TaskId, Timestamp,
    VersionSel,
};

pub use derive::{DerivationInput, DerivationProducer, DerivationRun, DerivationStep};
pub use options::{
    BLOB_DEDICATED_THRESHOLD, BLOB_INLINE_THRESHOLD, INLINE_TEXT_MAX, Options, STAGING_TTL,
    check_storage,
};
pub use strata_gateway::maintenance::{PrunePlan, PruneReport, StoreCounts};
pub use strata_gateway::task::{
    STATUS_CANCELLED, STATUS_DONE, STATUS_LEASED, STATUS_QUEUED, TaskFilter, TaskPage, TaskRow,
    TaskStats, known_reason, known_status,
};
pub use strata_gateway::{Backend, BlobBackend, DerivationSpec, DerivationTask, Tokenizer};

pub struct Strata {
    gateway: Gateway,
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

/// 内容库总览。`variants` 与 `sources` 已按条数降序。
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub total: u64,
    pub tombstoned: u64,
    pub variants: Vec<VariantTotal>,
    pub sources: Vec<SourceTotal>,
    /// 用户分类标签分布：按最新一条标记统计，墓碑不计入。
    pub labels: Vec<LabelTotal>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelTotal {
    pub key: String,
    pub value: String,
    pub total: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VariantTotal {
    pub variant: String,
    pub total: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceTotal {
    pub source_id: String,
    pub total: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Store(#[from] strata_gateway::Error),
    #[error("不支持的查询表达式")]
    UnsupportedExpr,
    #[error("派生声明无效: {0}")]
    InvalidDerivation(String),
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
        Self::Store(strata_gateway::Error::from(value))
    }
}

impl Strata {
    pub async fn open(root: impl AsRef<std::path::Path>, options: Options) -> Result<Self, Error> {
        Ok(Self {
            gateway: Gateway::open(root, options.gateway()).await?,
        })
    }

    pub async fn stage(&self, bytes: &[u8]) -> Result<PartRef, Error> {
        Ok(self.gateway.stage(bytes).await?)
    }

    /// 向量检索是否走 sqlite-vec/pgvector 的原生距离函数。
    pub fn vector_accelerated(&self) -> bool {
        self.gateway.vector_accelerated()
    }

    /// 元数据与 Blob 后端名字，供启动日志。
    pub fn backend_names(&self) -> (&'static str, &'static str) {
        self.gateway.backend_names()
    }

    pub async fn apply(
        &self,
        changeset: ChangeSet,
        opts: ApplyOpts,
    ) -> Result<CommitResult, Error> {
        Ok(self.gateway.apply(changeset, opts).await?)
    }

    pub async fn flush(&self) -> Result<Option<u64>, Error> {
        Ok(self.gateway.flush().await?)
    }

    pub async fn recover(&self) -> Result<(), Error> {
        Ok(self.gateway.recover().await?)
    }

    pub async fn kinds(
        &self,
        rid: &Rid,
        include_variants: bool,
    ) -> Result<Vec<FacetSummary>, Error> {
        let view = self.gateway.view_latest().await?;
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
        let view = self.gateway.view_latest().await?;
        Ok(view.facets.iter().any(|facet| {
            facet.rid == rid.as_str()
                && facet.kind == kind.as_str()
                && variant.is_none_or(|expected| facet.variant.as_deref() == Some(expected))
        }))
    }

    pub async fn get(&self, reference: &FacetRef, lazy: bool) -> Result<Option<Facet>, Error> {
        let view = self.gateway.view_latest().await?;
        let Some(facet) = find_facet(&view, reference) else {
            return Ok(None);
        };
        Ok(Some(load_facet(&self.gateway, &view, facet, lazy).await?))
    }

    pub async fn slice(&self, span: &SpanRef) -> Result<Bytes, Error> {
        let view = self.gateway.view_latest().await?;
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
            .gateway
            .read_blob_range(&versions, &sha, span.start, span.end - span.start)
            .await?;
        Ok(Bytes::from(bytes))
    }

    pub async fn resolve(&self, identity_key: &str) -> Result<Option<Rid>, Error> {
        let view = self.gateway.view_latest().await?;
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
        let view = self.gateway.view_latest().await?;
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
        let view = self.gateway.view_latest().await?;
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
        let view = self.gateway.view_latest().await?;
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
        let view = match &query.as_of {
            Some(AsOf::Tag(_)) => return Err(Error::TagUnspecified),
            Some(AsOf::Snapshot(seq)) => self.gateway.view_snapshot(*seq).await?,
            Some(AsOf::Time(time)) => self.gateway.view_time(*time).await?,
            None => self.gateway.view_latest().await?,
        };
        if query.search.is_some() {
            return self.search(&view, query).await;
        }
        let page = query::execute(&view, &query)?;
        Ok(QueryPage {
            rows: page.rows,
            next_cursor: page.next_cursor,
        })
    }

    /// 融合检索：过滤表达式下推给数据层，命中行再按同一快照补齐投影。
    async fn search(&self, view: &View, query: Query) -> Result<QueryPage, Error> {
        if query.cursor.is_some() {
            return Err(Error::QueryShape);
        }
        let Some(snapshot) = view.snapshot.as_ref() else {
            return Ok(QueryPage {
                rows: Vec::new(),
                next_cursor: None,
            });
        };
        let request = GatewayRequest {
            from: query.from,
            filter: query.r#where.clone(),
            has_facets: query.has_facets.clone(),
            search: query.search.clone(),
            as_of: Some(snapshot.seq),
            order_by: query.order_by.clone(),
            limit: query.limit,
            cursor: None,
            include_tombstoned: query.include_tombstoned,
        };
        let response = self.gateway.query(request).await?;
        let mut rows = Vec::with_capacity(response.rows.len());
        for hit in response.rows {
            let row = view
                .resources
                .iter()
                .find(|resource| resource.rid == hit.rid)
                .map(|resource| query::resource_value(view, resource))
                .unwrap_or(hit.row);
            rows.push(query::project(view, &query, row)?);
        }
        Ok(QueryPage {
            rows,
            next_cursor: response.next_cursor.map(|seq| seq.to_string()),
        })
    }

    pub async fn changes(
        &self,
        after: u64,
        limit: Option<u32>,
    ) -> Result<Vec<VisibleChange>, Error> {
        let rows = self.gateway.changes(after, limit).await?;
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
        let view = self.gateway.view_latest().await?;
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

    /// 某作用域下的全部最新状态，按 key 升序。
    ///
    /// 运行态一次取全量，调用方不必逐键问一次。
    pub async fn states(&self, scope: &str) -> Result<Vec<(String, StateValue)>, Error> {
        let view = self.gateway.view_latest().await?;
        let mut latest: BTreeMap<&str, &StateRow> = BTreeMap::new();
        for row in view.states.iter().filter(|row| row.scope == scope) {
            let entry = latest.entry(row.key.as_str()).or_insert(row);
            if row.seq > entry.seq {
                *entry = row;
            }
        }
        Ok(latest
            .into_iter()
            .map(|(key, row)| {
                (
                    key.to_string(),
                    StateValue {
                        value: row.value.clone(),
                        version: 1,
                    },
                )
            })
            .collect())
    }

    /// 内容库总览：一次遍历得出总数、墓碑数、类型与来源分布。
    ///
    /// 口径与查询一致：墓碑既不计入总数，也不计入类型与来源分布。
    pub async fn summary(&self) -> Result<Summary, Error> {
        let view = self.gateway.view_latest().await?;
        // 同一 rid 取 seq 最新的一条标记：墓碑与标签都以头部标记为准。
        let mut marks: HashMap<&str, &strata_gateway::MarkRow> = HashMap::new();
        for mark in &view.marks {
            marks
                .entry(mark.rid.as_str())
                .and_modify(|current| {
                    if mark.seq >= current.seq {
                        *current = mark;
                    }
                })
                .or_insert(mark);
        }
        let mut total = 0u64;
        let mut tombstoned = 0u64;
        let mut live: HashSet<&str> = HashSet::new();
        let mut sources: BTreeMap<&str, u64> = BTreeMap::new();
        let mut labels: BTreeMap<(&str, &str), u64> = BTreeMap::new();
        for resource in &view.resources {
            if marks
                .get(resource.rid.as_str())
                .is_some_and(|mark| mark.tombstone)
            {
                tombstoned += 1;
                continue;
            }
            total += 1;
            live.insert(resource.rid.as_str());
            if let Some(source_id) = resource.source_id.as_deref() {
                *sources.entry(source_id).or_default() += 1;
            }
            for item in marks
                .get(resource.rid.as_str())
                .map(|mark| mark.labels.as_slice())
                .unwrap_or_default()
            {
                if let Some((key, value)) = item.split_once('=') {
                    *labels.entry((key, value)).or_default() += 1;
                }
            }
        }
        // 同一资源的同一变体只算一次：视图里保留着全部历史版本。
        let mut seen: HashSet<(&str, &str)> = HashSet::new();
        let mut counts: BTreeMap<&str, u64> = BTreeMap::new();
        for facet in &view.facets {
            if facet.kind != "struct" || !live.contains(facet.rid.as_str()) {
                continue;
            }
            let Some(variant) = facet.variant.as_deref().filter(|name| !name.is_empty()) else {
                continue;
            };
            if !seen.insert((facet.rid.as_str(), variant)) {
                continue;
            }
            *counts.entry(variant).or_default() += 1;
        }
        let mut variants: Vec<VariantTotal> = counts
            .into_iter()
            .map(|(variant, total)| VariantTotal {
                variant: variant.to_string(),
                total,
            })
            .collect();
        variants.sort_by(|left, right| {
            right
                .total
                .cmp(&left.total)
                .then_with(|| left.variant.cmp(&right.variant))
        });
        let mut sources: Vec<SourceTotal> = sources
            .into_iter()
            .map(|(source_id, total)| SourceTotal {
                source_id: source_id.to_string(),
                total,
            })
            .collect();
        sources.sort_by(|left, right| {
            right
                .total
                .cmp(&left.total)
                .then_with(|| left.source_id.cmp(&right.source_id))
        });
        let mut labels: Vec<LabelTotal> = labels
            .into_iter()
            .map(|((key, value), total)| LabelTotal {
                key: key.to_string(),
                value: value.to_string(),
                total,
            })
            .collect();
        labels.sort_by(|left, right| {
            right
                .total
                .cmp(&left.total)
                .then_with(|| left.key.cmp(&right.key))
                .then_with(|| left.value.cmp(&right.value))
        });
        Ok(Summary {
            total,
            tombstoned,
            variants,
            sources,
            labels,
        })
    }

    pub async fn enqueue(&self, tasks: &[AccessTask]) -> Result<(), Error> {
        self.gateway.enqueue(tasks).await?;
        Ok(())
    }

    pub async fn lease(
        &self,
        queue: QueueName,
        count: usize,
        lease_for: Duration,
    ) -> Result<Vec<AccessTask>, Error> {
        Ok(self
            .gateway
            .lease(queue, count, lease_for, Timestamp::now())
            .await?)
    }

    pub async fn extend(&self, task_id: &TaskId, lease_for: Duration) -> Result<(), Error> {
        Ok(self
            .gateway
            .extend(task_id, lease_for, Timestamp::now())
            .await?)
    }

    pub async fn nack(
        &self,
        task_id: &TaskId,
        retry_at: Timestamp,
        error: &str,
    ) -> Result<(), Error> {
        Ok(self.gateway.nack(task_id, retry_at, error).await?)
    }

    /// 任务一页，供控制台查看队列状态。
    pub async fn list_tasks(
        &self,
        filter: &strata_gateway::task::TaskFilter,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<strata_gateway::task::TaskPage, Error> {
        Ok(self.gateway.list_tasks(filter, cursor, limit).await?)
    }

    /// 队列全景统计。
    pub async fn task_stats(&self) -> Result<strata_gateway::task::TaskStats, Error> {
        Ok(self.gateway.task_stats().await?)
    }

    /// 取消一个尚未终态的任务。
    pub async fn cancel_task(&self, task_id: &TaskId) -> Result<bool, Error> {
        Ok(self.gateway.cancel_task(task_id).await?)
    }

    /// 重试或恢复一个任务。
    pub async fn requeue_task(&self, task_id: &TaskId) -> Result<bool, Error> {
        Ok(self.gateway.requeue_task(task_id).await?)
    }

    /// 删除一个终态任务。
    pub async fn remove_task(&self, task_id: &TaskId) -> Result<bool, Error> {
        Ok(self.gateway.remove_task(task_id).await?)
    }

    /// 按条件清理终态任务。
    pub async fn purge_tasks(
        &self,
        filter: &strata_gateway::task::TaskFilter,
    ) -> Result<u64, Error> {
        Ok(self.gateway.purge_tasks(filter).await?)
    }

    /// 按条件计数，供分页页脚与清理确认。
    pub async fn count_tasks(
        &self,
        filter: &strata_gateway::task::TaskFilter,
    ) -> Result<u64, Error> {
        Ok(self.gateway.count_tasks(filter).await?)
    }

    /// 每队列 × 状态计数。
    pub async fn queue_counts(&self) -> Result<Vec<strata_gateway::task::QueueCounts>, Error> {
        Ok(self.gateway.queue_counts().await?)
    }

    /// 读取一条任务的完整内容，供装配层执行。
    pub async fn read_task(
        &self,
        task_id: &TaskId,
    ) -> Result<Option<strata_gateway::task::StoredTask>, Error> {
        Ok(self.gateway.read_task(task_id).await?)
    }

    /// 单条任务行，供新建后回显。
    pub async fn task_row(
        &self,
        task_id: &TaskId,
    ) -> Result<Option<strata_gateway::task::TaskRow>, Error> {
        Ok(self.gateway.task_row(task_id).await?)
    }

    pub async fn lease_id(
        &self,
        task_id: &TaskId,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<AccessTask, Error> {
        Ok(self.gateway.lease_id(task_id, lease_for, now).await?)
    }

    pub async fn release_leases(&self) -> Result<Vec<AccessTask>, Error> {
        Ok(self.gateway.release_leases().await?)
    }

    pub async fn leased_count(&self, queue: QueueName, now: Timestamp) -> Result<usize, Error> {
        Ok(self.gateway.leased_count(queue, now).await?)
    }

    pub async fn task_active(&self, source: &str, locator_key: &str) -> Result<bool, Error> {
        Ok(self.gateway.task_active(source, locator_key).await?)
    }

    pub async fn lease_ready(
        &self,
        source: &str,
        locator_key: &str,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<Option<AccessTask>, Error> {
        Ok(self
            .gateway
            .lease_ready(source, locator_key, lease_for, now)
            .await?)
    }

    pub async fn lease_follows(
        &self,
        queue: QueueName,
        count: usize,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<Vec<AccessTask>, Error> {
        Ok(self
            .gateway
            .lease_follows(queue, count, lease_for, now)
            .await?)
    }

    pub async fn accepted_state(
        &self,
        scope: &str,
        key: &str,
    ) -> Result<Option<StateValue>, Error> {
        let Some((version, value)) = self.gateway.accepted_state(scope, key).await? else {
            return Ok(None);
        };
        Ok(Some(StateValue {
            value: serde_json::from_str(&value).map_err(strata_gateway::Error::from)?,
            version,
        }))
    }

    /// 按计划裁掉过期行。
    pub async fn prune(
        &self,
        plan: &strata_gateway::maintenance::PrunePlan,
        dry_run: bool,
    ) -> Result<strata_gateway::maintenance::PruneReport, Error> {
        Ok(self.gateway.prune(plan, dry_run).await?)
    }

    /// 物理清空内容、索引、快照与对象。
    pub async fn purge_content(&self) -> Result<strata_gateway::PurgeReport, Error> {
        Ok(self.gateway.purge_content().await?)
    }

    /// 只清派生 facet、全文与向量索引。
    pub async fn purge_derived(&self) -> Result<strata_gateway::PurgeReport, Error> {
        Ok(self.gateway.purge_derived().await?)
    }

    /// 物理删除一批内容资源、派生数据与无引用对象。
    pub async fn purge_resources(
        &self,
        rids: &[Rid],
    ) -> Result<strata_gateway::PurgeReport, Error> {
        let ids: Vec<String> = rids.iter().map(|rid| rid.as_str().to_string()).collect();
        Ok(self.gateway.purge_resources(&ids).await?)
    }

    /// 清无引用的 blob 行与外部对象；`dry_run` 只统计。
    pub async fn purge_unreferenced_blobs(
        &self,
        dry_run: bool,
    ) -> Result<strata_gateway::PurgeReport, Error> {
        Ok(self.gateway.purge_unreferenced_blobs(dry_run).await?)
    }

    /// 各表行数。
    pub async fn store_counts(&self) -> Result<strata_gateway::maintenance::StoreCounts, Error> {
        Ok(self.gateway.store_counts().await?)
    }

    pub async fn hot_get(&self, kind: &str, key: &str) -> Result<Option<Value>, Error> {
        let Some(value) = self.gateway.hot_get(kind, key).await? else {
            return Ok(None);
        };
        Ok(Some(
            serde_json::from_str(&value).map_err(strata_gateway::Error::from)?,
        ))
    }

    pub async fn hot_put(&self, kind: &str, key: &str, value: &Value) -> Result<(), Error> {
        let text = serde_json::to_string(value).map_err(strata_gateway::Error::from)?;
        Ok(self.gateway.hot_put(kind, key, &text).await?)
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
    gateway: &Gateway,
    view: &View,
    facet: &FacetRecord,
    lazy: bool,
) -> Result<Facet, Error> {
    let value = match &facet.body_struct {
        Some(raw) => Some(serde_json::from_str(raw).map_err(strata_gateway::Error::from)?),
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
        Some(Bytes::from(gateway.read_blob(&versions, sha).await?))
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
