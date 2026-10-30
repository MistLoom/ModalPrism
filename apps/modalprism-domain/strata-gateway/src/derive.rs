//! 声明式派生：按配置声明“媒体 facet → 派生 facet”，入库可见后自动触发。
//!
//! 是否重算由三件事共同决定：输入 facet 的版本、产出方版本、配置代际与参数指纹。
//! 任一项与已存在的派生 facet 不符，就重新入队；任务按 `source_id + locator_key`
//! 去重，同一输入在同一产出方版本下不会重复排队。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use strata_model::{
    AccessTask, FacetKind, FacetRef, Locator, Producer, Rid, SourceId, SpanRef, TaskId, TaskReason,
    Timestamp, VersionSel,
};

use crate::config::DerivationSpec;
use crate::data::{Dataset, FacetRow};
use crate::db::Db;
use crate::error::{Error, Result};
use crate::task;

/// 写入 `AccessTask.spec` 的派生载荷。执行方据此构造 `PutFacet` 并带上血缘。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DerivationTask {
    pub spec: String,
    pub rid: String,
    /// 输入 facet 引用（`r_…@kind[:variant]#vN`）。
    pub input: String,
    /// 输入字节数，用于把输入表达成 `SpanRef` 血缘。
    pub input_size: u64,
    pub produces: String,
    pub produces_variant: Option<String>,
    pub params_hash: String,
    pub producer: Producer,
    /// 触发时钉住的快照，执行方读数据以它为准。
    pub snapshot: u64,
}

/// (rid, kind, variant) → 最新一版 facet。
fn heads(dataset: &Dataset) -> BTreeMap<(String, String, String), FacetRow> {
    let mut heads: BTreeMap<(String, String, String), FacetRow> = BTreeMap::new();
    for facet in &dataset.facets {
        let key = (
            facet.rid.clone(),
            facet.kind.clone(),
            facet.variant.clone().unwrap_or_default(),
        );
        match heads.get(&key) {
            Some(current) if current.version >= facet.version => {}
            _ => {
                heads.insert(key, facet.clone());
            }
        }
    }
    heads
}

fn facet_ref(facet: &FacetRow) -> Result<FacetRef> {
    let kind = FacetKind::parse(&facet.kind)?;
    Ok(FacetRef::new(
        Rid::parse(&facet.rid)?,
        kind,
        facet.variant.clone(),
        VersionSel::Exact(facet.version),
    )?)
}

fn has_fields(facet: &FacetRow, fields: &[String]) -> bool {
    fields.iter().all(|field| match field.as_str() {
        "text" => facet.body_text.is_some(),
        "struct" => facet.body_struct.is_some(),
        "blob" => facet.blob_ref.is_some(),
        _ => false,
    })
}

/// 派生 facet 的血缘是否正好覆盖这一版输入。
fn covers_input(derived: &FacetRow, input: &FacetRow) -> bool {
    derived.inputs.iter().any(|raw| {
        let parsed = SpanRef::parse(raw)
            .map(|span| span.facet)
            .or_else(|_| FacetRef::parse(raw));
        match parsed {
            Ok(facet) => {
                facet.rid.as_str() == input.rid.as_str()
                    && facet.kind.as_str() == input.kind.as_str()
                    && facet.variant == input.variant
                    && matches!(facet.version, VersionSel::Exact(version) if version == input.version)
            }
            Err(_) => false,
        }
    })
}

/// 为一个快照生成派生任务并入队。返回真正入队的任务。
pub async fn trigger(
    db: &Db,
    specs: &[DerivationSpec],
    dataset: &Dataset,
    snapshot: u64,
) -> Result<Vec<AccessTask>> {
    if specs.is_empty() {
        return Ok(Vec::new());
    }
    let heads = heads(dataset);
    let mut tasks = Vec::new();
    for spec in specs {
        let params_hash = spec.params_hash();
        for ((rid, kind, variant), input) in &heads {
            if kind.as_str() != spec.from.kind.as_str() {
                continue;
            }
            if let Some(wanted) = &spec.from.variant {
                if variant.as_str() != wanted.as_str() {
                    continue;
                }
            }
            if !has_fields(input, &spec.from.fields) {
                continue;
            }
            let derived_key = (
                rid.clone(),
                spec.produces.as_str().to_string(),
                spec.variant_key(),
            );
            if let Some(derived) = heads.get(&derived_key) {
                let fresh = derived.params_hash == params_hash
                    && derived.producer_module == spec.producer.module
                    && derived.producer_version == spec.producer.version
                    && derived.config_generation == spec.producer.config_generation
                    && covers_input(derived, input);
                if fresh {
                    continue;
                }
            }
            let input_ref = facet_ref(input)?;
            let locator = format!(
                "strata://derive/{}/{}@{}:{}#v{}",
                spec.name, rid, kind, variant, input.version
            );
            let payload = DerivationTask {
                spec: spec.name.clone(),
                rid: rid.clone(),
                input: input_ref.to_string(),
                input_size: input.size,
                produces: spec.produces.as_str().to_string(),
                produces_variant: spec.variant.clone(),
                params_hash: params_hash.clone(),
                producer: spec.producer.clone(),
                snapshot,
            };
            tasks.push(AccessTask {
                task_id: TaskId::new(ulid::Ulid::new().to_string())?,
                source_id: SourceId::new(spec.name.clone())?,
                plan_id: None,
                accessor: spec.accessor_name().to_string(),
                locator: Locator::Query { query: locator },
                spec: serde_json::to_value(&payload)?,
                reason: TaskReason::Detail,
                priority: spec.priority,
                queue: spec.queue,
                lease_until: Timestamp::now(),
                attempt: 0,
                batch_key: None,
                generation: spec.producer.config_generation,
            });
        }
    }
    task::enqueue(db, &tasks).await
}

/// 解析派生载荷。
pub fn parse_task(spec: &serde_json::Value) -> Result<DerivationTask> {
    serde_json::from_value(spec.clone()).map_err(Error::from)
}
