//! 执行一个已租约的任务。访问器产出信封，空 pipeline 写成 raw facet。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use accessors::{AccessContext, AccessHints, AccessRequest, Accessor, Session, SessionPool};
use accessors_builtin::{BrowserAccessor, MoliCommand, by_id};
use adapters::{
    Admission, DetectorChoice, FollowRequest, PartBytes, Registry, empty_identities,
    parse_pipeline, run, source_view,
};
use async_trait::async_trait;
use bytes::Bytes;
use config::Snapshot;
use planner::{Classified, Quality, ReceiptBody, TaskSuccess};
use serde_json::{Map, Value};
use strata_model::{
    AccessTask, Body, ErrorClass, FacetKind, IdentityKey, LineageInput, Locator, Op, PartRole,
    Producer, QueueName, RawEnvelope, Rid, SourceId, Target, TaskId, TaskReason, Timestamp,
};

use crate::stage::{DiskStaging, read_part};
use crate::transport::{EgressGw, PaidGw};

pub struct FetchState {
    pub snapshot: Arc<Snapshot>,
    pub egress: Arc<EgressGw>,
    pub paid: Arc<PaidGw>,
    pub staging_dir: PathBuf,
    pub proxy: String,
    pub browser: PathBuf,
    pub timeout: std::time::Duration,
}

pub(crate) struct NoSession;

#[async_trait]
impl SessionPool for NoSession {
    async fn get(&self, _profile: &str) -> Result<Session, accessors::AccessError> {
        Err(accessors::AccessError::new(ErrorClass::Config, "session"))
    }
}

pub(crate) struct FollowWork {
    pub task: AccessTask,
    pub identity: String,
    pub needs: Vec<String>,
}

pub(crate) struct Fetched {
    pub success: TaskSuccess,
    pub follows: Vec<FollowWork>,
    pub max_fetches: usize,
}

pub async fn fetch(
    state: &FetchState,
    task: &strata_model::AccessTask,
    cursor: Option<Value>,
    resource: Option<Rid>,
) -> Result<Fetched, Classified> {
    let module = state
        .snapshot
        .modules
        .get(task.source_id.as_str())
        .cloned()
        .ok_or_else(|| classified(ErrorClass::Config, "config"))?;
    let accessor_id = if task.reason == TaskReason::Follow {
        task.accessor.as_str()
    } else {
        module
            .pointer("/access/accessor")
            .and_then(Value::as_str)
            .unwrap_or(task.accessor.as_str())
    };
    let implementation = state
        .snapshot
        .modules
        .get(accessor_id)
        .and_then(|item| item.get("implementation"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let accessor = open_accessor(implementation, &state.browser, &state.proxy)?;
    let source_policy = module
        .get("policy")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    let accessor_policy = state
        .snapshot
        .modules
        .get(accessor_id)
        .and_then(|item| item.get("policy"))
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    let policy = config::overlay_system_policy(&state.snapshot.system, &source_policy);
    let egress_policy = crate::transport::policy_from(&state.snapshot.system, &source_policy);
    let paid_policy = crate::transport::call_policy_from(&state.snapshot.system, &accessor_policy);
    let source_proxy = egress::setting_of(&policy);
    let spec = if task.spec.is_object() {
        task.spec.clone()
    } else {
        Value::Object(Map::new())
    };
    let initial = cursor.is_none();
    let request = AccessRequest {
        task_id: task.task_id.clone(),
        source_id: task.source_id.clone(),
        locator: task.locator.clone(),
        spec,
        cursor,
        hints: AccessHints::default(),
        policy: Arc::new(policy),
        deadline: Instant::now() + state.timeout,
    };
    let context = AccessContext {
        egress: Arc::new(crate::transport::BoundEgress {
            inner: Arc::clone(&state.egress),
            policy: egress_policy.clone(),
        }),
        paid: Arc::new(crate::transport::BoundPaid {
            inner: Arc::clone(&state.paid),
            policy: paid_policy,
        }),
        sessions: Arc::new(NoSession),
        staging: Arc::new(DiskStaging {
            dir: state.staging_dir.clone(),
        }),
        cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    let deadline = request.deadline;
    let envelopes = crate::transport::REQUEST_DEADLINE
        .scope(
            deadline,
            crate::transport::SOURCE_PROXY.scope(
                source_proxy,
                crate::transport::EGRESS_POLICY
                    .scope(egress_policy, accessor.access(request, &context)),
            ),
        )
        .await
        .map_err(from_access)?;
    let pipeline = module
        .get("pipeline")
        .cloned()
        .unwrap_or(Value::Array(Vec::new()));
    let (ops, follow_requests) =
        if pipeline.as_array().is_some_and(Vec::is_empty) || pipeline.is_null() {
            (raw_ops(&envelopes)?, Vec::new())
        } else {
            adapted_ops(
                state,
                task,
                &module,
                &envelopes,
                &pipeline,
                initial,
                resource.as_ref(),
            )
            .await?
        };
    let max_fetches = detail_budget(&module);
    let follows = if task.reason == TaskReason::Schedule && max_fetches > 0 {
        follow_tasks(task, &follow_requests)?
    } else {
        Vec::new()
    };
    let new_resources = ops
        .iter()
        .filter(|op| matches!(op, Op::Ensure { .. }))
        .count() as u64;
    let last = envelopes.last();
    let success = TaskSuccess {
        ops,
        cursor: last.and_then(|item| item.cursor_candidate.clone()),
        receipt: match last {
            Some(envelope) => receipt_of(&state.paid, envelope).await?,
            None => None,
        },
        quality: Quality::Ok,
        selector_miss: false,
        blocked: false,
        ttl: last.and_then(|item| item.signals.ttl),
        retry_after: last.and_then(|item| item.signals.retry_after),
        cache_max_age: last.and_then(|item| item.signals.cache_max_age),
        expires: None,
        new_resources,
        render_level: last
            .and_then(|item| item.transport.render_level)
            .map(render_name)
            .map(str::to_string),
        cost: 0,
        adapters: Vec::new(),
        follows: Vec::new(),
    };
    Ok(Fetched {
        success,
        follows,
        max_fetches,
    })
}

fn render_name(level: strata_model::RenderLevel) -> &'static str {
    match level {
        strata_model::RenderLevel::L0 => "L0",
        strata_model::RenderLevel::L1 => "L1",
        strata_model::RenderLevel::L2 => "L2",
        strata_model::RenderLevel::L3 => "L3",
        strata_model::RenderLevel::L4 => "L4",
    }
}

pub(crate) fn open_accessor(
    implementation: &str,
    browser: &std::path::Path,
    proxy: &str,
) -> Result<Box<dyn Accessor>, Classified> {
    let Some(name) = implementation.strip_prefix("builtin:") else {
        return Err(classified(ErrorClass::Config, "implementation"));
    };
    if name == "browser" {
        return Ok(Box::new(BrowserAccessor::new(
            MoliCommand {
                program: browser.to_path_buf(),
            },
            proxy,
        )));
    }
    by_id(name).ok_or_else(|| classified(ErrorClass::Config, "implementation"))
}

pub(crate) async fn inbound_ops(
    state: &FetchState,
    source_id: &str,
    module: &Value,
    envelopes: &[RawEnvelope],
) -> Result<Vec<Op>, Classified> {
    let source_id =
        SourceId::new(source_id).map_err(|_| classified(ErrorClass::Config, "config"))?;
    let accessor = module
        .pointer("/access/accessor")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let channel = module
        .pointer("/access/locator/channel")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let task = strata_model::AccessTask {
        task_id: TaskId::new(format!("push-{source_id}"))
            .map_err(|_| classified(ErrorClass::Config, "config"))?,
        source_id,
        plan_id: None,
        accessor,
        locator: Locator::PushChannel { channel },
        spec: Value::Object(Map::new()),
        reason: TaskReason::Manual,
        priority: 0,
        queue: QueueName::Push,
        lease_until: Timestamp::now(),
        attempt: 1,
        batch_key: None,
        generation: state.snapshot.generation,
    };
    let pipeline = module
        .get("pipeline")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    if pipeline.as_array().is_some_and(Vec::is_empty) || pipeline.is_null() {
        let mut ops = raw_ops(envelopes)?;
        stamp_source(&mut ops, task.source_id.as_str());
        Ok(ops)
    } else {
        adapted_ops(state, &task, module, envelopes, &pipeline, false, None)
            .await
            .map(|(ops, _)| ops)
    }
}

fn raw_ops(envelopes: &[RawEnvelope]) -> Result<Vec<Op>, Classified> {
    let mut ops = Vec::new();
    for (index, envelope) in envelopes.iter().enumerate() {
        let Some(part) = envelope
            .parts
            .iter()
            .find(|part| part.role == PartRole::Document)
        else {
            continue;
        };
        let located = envelope
            .final_locator
            .clone()
            .unwrap_or_else(|| envelope.locator.clone());
        let url = match located {
            Locator::Url { url } => url,
            Locator::Query { query } => query,
            Locator::ApiEndpoint { endpoint } => endpoint,
            Locator::PushChannel { channel } => format!("push://{channel}"),
        };
        let identity = format!("url:{url}");
        IdentityKey::parse(&identity).map_err(|_| classified(ErrorClass::Permanent, "identity"))?;
        let alias = format!("p{index}");
        ops.push(Op::Ensure {
            identity,
            ns: None,
            labels: BTreeMap::new(),
            r#as: alias.clone(),
        });
        ops.push(Op::PutFacet {
            target: Target::Alias(alias),
            kind: FacetKind::Raw,
            variant: None,
            body: Body::Staged {
                part: part.bytes.clone(),
            },
            media_type: Some(part.media_type.clone()),
            lineage: LineageInput {
                inputs: Vec::new(),
                params_hash: "raw".into(),
            },
        });
    }
    Ok(ops)
}

/// R1–R3：把 pipeline 里的 `binding: "binding/x"` 引用在运行期展开为内联步骤。
///
/// 信源只声明「用哪个绑定」，不再内嵌可执行 spec；绑定的适配器与装配参数（`params`，
/// 回退 `spec`）在此从快照取出。条件（`when`）、`for`、`after` 仍留在引用侧，绑定只承载
/// 装配。展开不改动信源字节——`config_hash` 已在 R11.1 排除 pipeline/binding，故迁移不抖动。
/// 绑定缺失 / 无 adapter 视为配置错误（`check` 已在落盘期挡住悬空引用，这里是运行期兜底）。
fn expand_pipeline_bindings(pipeline: &Value, snapshot: &Snapshot) -> Result<Value, Classified> {
    let Some(steps) = pipeline.as_array() else {
        return Ok(pipeline.clone());
    };
    // 没有绑定引用时原样返回，保证旧内联 pipeline 行为逐字节不变。
    if !steps
        .iter()
        .any(|step| step.get("binding").and_then(Value::as_str).is_some())
    {
        return Ok(pipeline.clone());
    }
    let mut out = Vec::with_capacity(steps.len());
    for step in steps {
        let Some(obj) = step.as_object() else {
            out.push(step.clone());
            continue;
        };
        let Some(binding_id) = obj.get("binding").and_then(Value::as_str) else {
            out.push(step.clone());
            continue;
        };
        let binding = snapshot
            .modules
            .get(binding_id)
            .ok_or_else(|| classified(ErrorClass::Config, format!("绑定不存在: {binding_id}")))?;
        let adapter_id = binding
            .get("adapter")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                classified(
                    ErrorClass::Config,
                    format!("绑定缺少 adapter: {binding_id}"),
                )
            })?;
        let params = binding
            .get("params")
            .filter(|value| value.is_object())
            .or_else(|| binding.get("spec").filter(|value| value.is_object()))
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let mut expanded = Map::new();
        for key in ["when", "for", "after"] {
            if let Some(value) = obj.get(key) {
                expanded.insert(key.to_string(), value.clone());
            }
        }
        expanded.insert(
            "run".into(),
            Value::Array(vec![Value::String(adapter_id.to_string())]),
        );
        let mut spec = Map::new();
        spec.insert(adapter_id.to_string(), params);
        expanded.insert("spec".into(), Value::Object(spec));
        out.push(Value::Object(expanded));
    }
    Ok(Value::Array(out))
}

async fn adapted_ops(
    state: &FetchState,
    task: &strata_model::AccessTask,
    module: &Value,
    envelopes: &[RawEnvelope],
    pipeline: &Value,
    initial: bool,
    resource: Option<&Rid>,
) -> Result<(Vec<Op>, Vec<FollowRequest>), Classified> {
    let pipeline = expand_pipeline_bindings(pipeline, &state.snapshot)?;
    let rules =
        parse_pipeline(&pipeline).map_err(|error| classified(error.class(), error.to_string()))?;
    let mut parts = Vec::new();
    for envelope in envelopes {
        for part in &envelope.parts {
            let bytes = read_part(&state.staging_dir, &part.bytes).map_err(from_access)?;
            parts.push(PartBytes {
                part_id: part.part_id.clone(),
                role: part.role.clone(),
                declared: part.media_type.clone(),
                charset: part.charset.clone(),
                bytes: Bytes::from(bytes),
                facet: None,
            });
        }
    }
    let mut specs = Map::new();
    for (id, item) in &state.snapshot.modules {
        if item.get("kind").and_then(Value::as_str) == Some("adapter") {
            if let Some(spec) = item.get("spec").cloned() {
                specs.insert(id.clone(), spec);
            }
        }
    }
    let admission = Admission::from_value(&merged_admission(&state.snapshot, task, module))
        .map_err(|error| classified(error.class(), error.to_string()))?;
    let output = run(
        &Registry::builtins(),
        adapters::RunRequest {
            parts,
            pipeline: rules,
            reason: task.reason,
            source: source_view(task.source_id.as_str(), accessor_of(task, module).as_str())
                .map_err(|error| classified(error.class(), error.to_string()))?,
            resource: resource.cloned(),
            origin: None,
            locator: task.locator.clone(),
            detector: DetectorChoice::Basic,
            admission,
            now: strata_model::Timestamp::now(),
            initial,
            producer: Producer {
                module: "runner".into(),
                version: "1".into(),
                config_generation: task.generation,
            },
            specs,
            identities: empty_identities(),
        },
    )
    .await
    .map_err(|error| classified(error.class(), error.to_string()))?;
    let mut ops = output.changeset.ops;
    stamp_source(&mut ops, task.source_id.as_str());
    Ok((ops, output.follows))
}

/// 给 `Ensure` 补 `source` 标签。
///
/// 内容身份用 url/ext，跨信源去重，来源因此不属于身份键；网关把该标签落成
/// `resources.source_id`，内容库才能按项目可见信源过滤。已有标签不覆盖：同一 URL
/// 被多个信源采集时归属首个写入者。
fn stamp_source(ops: &mut [Op], source: &str) {
    for op in ops {
        if let Op::Ensure { labels, .. } = op {
            labels
                .entry("source".to_string())
                .or_insert_with(|| source.to_string());
        }
    }
}

fn accessor_of(task: &AccessTask, module: &Value) -> String {
    if task.reason == TaskReason::Follow {
        return task.accessor.clone();
    }
    module
        .pointer("/access/accessor")
        .and_then(Value::as_str)
        .unwrap_or(task.accessor.as_str())
        .to_string()
}

/// 找到任务所属计划的 `quota`（计划化后额度不再写在信源上）。
fn plan_quota(snapshot: &Snapshot, task: &AccessTask) -> Option<Value> {
    let plan_id = task.plan_id.as_deref()?;
    snapshot
        .modules
        .get(plan_id)?
        .get("quota")
        .cloned()
        .filter(|value| !value.is_null())
}

/// 回灌计划额度到 `Admission` 输入：`quota.perRunItems → initialBackfill.limit`、
/// `quota.maxAge → initialBackfill.maxAge`，使迁出信源的回填/时间窗治理仍在执行侧生效。
/// 信源若仍自带 `initialBackfill`（未迁移），保持不变。
fn merged_admission(snapshot: &Snapshot, task: &AccessTask, module: &Value) -> Value {
    let mut admission = module
        .get("admission")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    let Some(map) = admission.as_object_mut() else {
        return admission;
    };
    if map.contains_key("initialBackfill") {
        return admission;
    }
    let Some(quota) = plan_quota(snapshot, task) else {
        return admission;
    };
    let per_run = quota.get("perRunItems").cloned().filter(|v| !v.is_null());
    let max_age = quota.get("maxAge").cloned().filter(|v| !v.is_null());
    if per_run.is_some() || max_age.is_some() {
        let mut backfill = Map::new();
        if let Some(limit) = per_run {
            backfill.insert("limit".into(), limit);
        }
        if let Some(age) = max_age {
            backfill.insert("maxAge".into(), age);
        }
        map.insert("initialBackfill".into(), Value::Object(backfill));
    }
    admission
}

fn detail_budget(module: &Value) -> usize {
    let Some(steps) = module.get("pipeline").and_then(Value::as_array) else {
        return 0;
    };
    let mut max = 0u64;
    for step in steps {
        let Some(spec) = step.get("spec").and_then(Value::as_object) else {
            continue;
        };
        for value in spec.values() {
            if let Some(count) = value.pointer("/detail/maxFetches").and_then(Value::as_u64) {
                max = max.max(count);
            }
        }
    }
    usize::try_from(max).unwrap_or(0)
}

fn follow_tasks(
    task: &AccessTask,
    follows: &[FollowRequest],
) -> Result<Vec<FollowWork>, Classified> {
    let mut out = Vec::new();
    for follow in follows {
        if follow.needs.is_empty() {
            continue;
        }
        let seq = FOLLOW_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let task_id = TaskId::new(format!("t_follow_{seq}"))
            .map_err(|_| classified(ErrorClass::Permanent, "identity"))?;
        out.push(FollowWork {
            task: AccessTask {
                task_id,
                source_id: task.source_id.clone(),
                plan_id: task.plan_id.clone(),
                accessor: follow.accessor.clone(),
                locator: follow.locator.clone(),
                spec: follow.spec.clone(),
                reason: TaskReason::Follow,
                priority: 20,
                queue: QueueName::Free,
                lease_until: Timestamp::now(),
                attempt: 0,
                batch_key: None,
                generation: task.generation,
            },
            identity: follow.identity.clone(),
            needs: follow.needs.clone(),
        });
    }
    Ok(out)
}

static FOLLOW_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

async fn receipt_of(
    paid: &PaidGw,
    envelope: &RawEnvelope,
) -> Result<Option<ReceiptBody>, Classified> {
    let Some(id) = envelope.transport.receipt_id.as_ref() else {
        return Ok(None);
    };
    let Some((service, logical_key)) = paid
        .stored_receipt(id.as_str())
        .await
        .map_err(from_access)?
    else {
        return Err(classified(ErrorClass::Transient, "unknown-receipt"));
    };
    Ok(Some(ReceiptBody {
        id: id.as_str().to_string(),
        service,
        logical_key,
    }))
}

fn from_access(error: accessors::AccessError) -> Classified {
    Classified {
        class: error.class,
        message: error.message,
        retry_after: None,
        retry_at: error.retry_at,
    }
}

fn classified(class: ErrorClass, message: impl Into<String>) -> Classified {
    Classified {
        class,
        message: message.into(),
        retry_after: None,
        retry_at: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snapshot_with(modules: Vec<(String, Value)>) -> Snapshot {
        Snapshot {
            generation: 1,
            system: json!({}),
            modules: modules.into_iter().collect(),
            origins: Default::default(),
        }
    }

    /// R1–R3：绑定引用被展开为对适配器的内联步骤，params 成为 spec overlay。
    #[test]
    fn expands_binding_reference() {
        let snapshot = snapshot_with(vec![(
            "binding/feed".into(),
            json!({
                "kind": "binding",
                "adapter": "adapter/json-list",
                "params": {"items": "/data", "url": "link"}
            }),
        )]);
        let pipeline = json!([{ "binding": "binding/feed", "for": "follow" }]);
        let expanded = expand_pipeline_bindings(&pipeline, &snapshot).unwrap();
        let step = &expanded[0];
        assert_eq!(
            step["run"].as_array().unwrap()[0],
            json!("adapter/json-list")
        );
        assert_eq!(step["spec"]["adapter/json-list"]["items"], json!("/data"));
        assert_eq!(step["for"], json!("follow"));
        assert!(step.get("binding").is_none(), "展开后不应残留 binding 引用");
    }

    /// 无绑定引用时原样返回，保证旧内联 pipeline 逐字节不变。
    #[test]
    fn leaves_inline_pipeline_untouched() {
        let snapshot = snapshot_with(Vec::new());
        let pipeline =
            json!([{"run": ["adapter/html-list"], "spec": {"adapter/html-list": {"item": "a"}}}]);
        let expanded = expand_pipeline_bindings(&pipeline, &snapshot).unwrap();
        assert_eq!(expanded, pipeline);
    }

    /// 悬空绑定引用是配置错误（落盘期 check 已拦截，这里是运行期兜底）。
    #[test]
    fn missing_binding_is_config_error() {
        let snapshot = snapshot_with(Vec::new());
        let pipeline = json!([{ "binding": "binding/ghost" }]);
        assert!(expand_pipeline_bindings(&pipeline, &snapshot).is_err());
    }
}
