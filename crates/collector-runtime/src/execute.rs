//! 执行一个已租约的任务。访问器产出信封，空 pipeline 写成 raw facet。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use accessors::{AccessContext, AccessHints, AccessRequest, Accessor, Session, SessionPool};
use accessors_builtin::{BrowserAccessor, MoliCommand, by_id};
use adapters::{
    Admission, AdapterFeedback, DetectorChoice, FollowRequest, PartBytes, Quality as AdapterQuality,
    Registry, empty_identities, parse_pipeline, run, source_view,
};
use async_trait::async_trait;
use bytes::Bytes;
use config::Snapshot;
use planner::{Classified, Quality, TaskSuccess};
use serde_json::{Map, Value};
use strata_model::{
    AccessTask, Body, ErrorClass, FacetKind, IdentityKey, LineageInput, Locator, Op, PartRole,
    Producer, QueueName, RawEnvelope, Rid, SourceId, Target, TaskId, TaskReason, Timestamp,
};

use collector_gateway::{REQUEST_DEADLINE, EGRESS_POLICY};
use egress::Policy;

use crate::stage::read_part;

/// 任务抓取所需的非网关环境。
pub struct FetchEnv {
    pub snapshot: Arc<Snapshot>,
    pub staging_dir: PathBuf,
    pub proxy: String,
    pub browser: PathBuf,
    pub timeout: std::time::Duration,
}

pub struct NoSession;

#[async_trait]
impl SessionPool for NoSession {
    async fn get(&self, _profile: &str) -> Result<Session, accessors::AccessError> {
        Err(accessors::AccessError::new(ErrorClass::Config, "session"))
    }
}

pub struct FollowWork {
    pub task: AccessTask,
    pub identity: String,
    pub needs: Vec<String>,
}

pub struct Fetched {
    pub success: TaskSuccess,
    pub follows: Vec<FollowWork>,
    pub max_fetches: usize,
}

pub async fn fetch(
    env: &FetchEnv,
    accessor: Box<dyn Accessor>,
    access: &AccessContext,
    egress_policy: Policy,
    task: &strata_model::AccessTask,
    cursor: Option<Value>,
    resource: Option<Rid>,
) -> Result<Fetched, Classified> {
    let module = env
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
    let _implementation = env
        .snapshot
        .modules
        .get(accessor_id)
        .and_then(|item| item.get("implementation"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let source_policy = module
        .get("policy")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    let _accessor_policy = env
        .snapshot
        .modules
        .get(accessor_id)
        .and_then(|item| item.get("policy"))
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    let policy = config::overlay_system_policy(&env.snapshot.system, &source_policy);
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
        deadline: Instant::now() + env.timeout,
    };
    let deadline = request.deadline;
    let envelopes = REQUEST_DEADLINE
        .scope(
            deadline,
            EGRESS_POLICY
                .scope(egress_policy, accessor.access(request, access)),
        )
        .await
        .map_err(from_access)?;
    let pipeline = module
        .get("pipeline")
        .cloned()
        .unwrap_or(Value::Array(Vec::new()));
    let (ops, follow_requests, quality, selector_miss, blocked) =
        if pipeline.as_array().is_some_and(Vec::is_empty) || pipeline.is_null() {
            let ops = raw_ops(&envelopes)?;
            let quality = if envelopes.is_empty() {
                Quality::Empty
            } else {
                Quality::Ok
            };
            (ops, Vec::new(), quality, false, false)
        } else {
            adapted_ops(
                env,
                task,
                &module,
                &envelopes,
                &pipeline,
                initial,
                resource.as_ref(),
            )
            .await?
        };
    schedule_ingest_error(task, &envelopes, &ops, quality, blocked)?;
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
            Some(_envelope) => None,
            None => None,
        },
        quality,
        selector_miss,
        blocked,
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

pub fn open_accessor(
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

pub async fn inbound_ops(
    env: &FetchEnv,
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
        accessor,
        locator: Locator::PushChannel { channel },
        spec: Value::Object(Map::new()),
        reason: TaskReason::Manual,
        priority: 0,
        queue: QueueName::Push,
        lease_until: Timestamp::now(),
        attempt: 1,
        batch_key: None,
        generation: env.snapshot.generation,
    };
    let pipeline = module
        .get("pipeline")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    if pipeline.as_array().is_some_and(Vec::is_empty) || pipeline.is_null() {
        raw_ops(envelopes)
    } else {
        adapted_ops(env, &task, module, envelopes, &pipeline, false, None)
            .await
            .map(|(ops, _, _, _, _)| ops)
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

async fn adapted_ops(
    env: &FetchEnv,
    task: &strata_model::AccessTask,
    module: &Value,
    envelopes: &[RawEnvelope],
    pipeline: &Value,
    initial: bool,
    resource: Option<&Rid>,
) -> Result<(Vec<Op>, Vec<FollowRequest>, Quality, bool, bool), Classified> {
    let rules =
        parse_pipeline(pipeline).map_err(|error| classified(error.class(), error.to_string()))?;
    let mut parts = Vec::new();
    for envelope in envelopes {
        for part in &envelope.parts {
            let bytes = read_part(&env.staging_dir, &part.bytes).map_err(from_access)?;
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
    for (id, item) in &env.snapshot.modules {
        if item.get("kind").and_then(Value::as_str) == Some("adapter") {
            if let Some(spec) = item.get("spec").cloned() {
                specs.insert(id.clone(), spec);
            }
        }
    }
    let admission = Admission::from_value(module.get("admission").unwrap_or(&Value::Null))
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
                source_id: Some(task.source_id.as_str().to_string()),
            },
            specs,
            identities: empty_identities(),
        },
    )
    .await
    .map_err(|error| classified(error.class(), error.to_string()))?;
    let (quality, blocked, selector_miss) = feedback_flags(&output.feedback);
    Ok((output.changeset.ops, output.follows, quality, selector_miss, blocked))
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

fn feedback_flags(feedback: &[AdapterFeedback]) -> (Quality, bool, bool) {
    let mut quality = Quality::Ok;
    let mut blocked = false;
    let mut selector_miss = false;
    for item in feedback {
        if !item.feedback.missing.is_empty() {
            selector_miss = true;
        }
        match item.feedback.quality {
            AdapterQuality::Blocked => {
                blocked = true;
                quality = Quality::Blocked;
            }
            AdapterQuality::Empty => {
                if quality != Quality::Blocked {
                    quality = Quality::Empty;
                }
            }
            AdapterQuality::Thin => {
                if matches!(quality, Quality::Ok) {
                    quality = Quality::Thin;
                }
            }
            AdapterQuality::Ok => {}
        }
    }
    (quality, blocked, selector_miss)
}

pub fn schedule_ingest_error(
    task: &AccessTask,
    envelopes: &[RawEnvelope],
    ops: &[Op],
    quality: Quality,
    blocked: bool,
) -> Result<(), Classified> {
    if task.reason != TaskReason::Schedule || !ops.is_empty() {
        return Ok(());
    }
    if blocked || quality == Quality::Blocked {
        return Err(classified(ErrorClass::Permanent, "blocked"));
    }
    if envelopes.is_empty() || quality == Quality::Empty {
        return Err(classified(ErrorClass::Permanent, "empty-fetch"));
    }
    Ok(())
}
