//! 执行一个已租约的任务。访问器产出信封，空 pipeline 写成 raw facet。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use accessors::{AccessContext, AccessHints, AccessRequest, Accessor, Session, SessionPool};
use accessors_builtin::{BrowserAccessor, MoliCommand, by_id};
use adapters::{
    Admission, DetectorChoice, PartBytes, Registry, empty_identities, parse_pipeline, run,
    source_view,
};
use async_trait::async_trait;
use bytes::Bytes;
use config::Snapshot;
use planner::{Classified, Quality, ReceiptBody, TaskSuccess};
use serde_json::{Map, Value};
use strata_model::{
    Body, ErrorClass, FacetKind, IdentityKey, LineageInput, Locator, Op, PartRole, Producer,
    RawEnvelope, Target, TaskReason,
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

pub async fn fetch(
    state: &FetchState,
    task: &strata_model::AccessTask,
    cursor: Option<Value>,
) -> Result<TaskSuccess, Classified> {
    let module = state
        .snapshot
        .modules
        .get(task.source_id.as_str())
        .cloned()
        .ok_or_else(|| classified(ErrorClass::Config, "config"))?;
    let accessor_id = module
        .pointer("/access/accessor")
        .and_then(Value::as_str)
        .unwrap_or(task.accessor.as_str());
    let implementation = state
        .snapshot
        .modules
        .get(accessor_id)
        .and_then(|item| item.get("implementation"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let accessor = open_accessor(implementation, &state.browser, &state.proxy)?;
    let policy = module
        .get("policy")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
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
        egress: state.egress.clone(),
        paid: state.paid.clone(),
        sessions: Arc::new(NoSession),
        staging: Arc::new(DiskStaging {
            dir: state.staging_dir.clone(),
        }),
        cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    let envelopes = accessor
        .access(request, &context)
        .await
        .map_err(from_access)?;
    let pipeline = module
        .get("pipeline")
        .cloned()
        .unwrap_or(Value::Array(Vec::new()));
    let ops = if pipeline.as_array().is_some_and(Vec::is_empty) || pipeline.is_null() {
        raw_ops(&envelopes)?
    } else {
        adapted_ops(state, task, &module, &envelopes, &pipeline, initial).await?
    };
    let new_resources = ops
        .iter()
        .filter(|op| matches!(op, Op::Ensure { .. }))
        .count() as u64;
    let last = envelopes.last();
    Ok(TaskSuccess {
        ops,
        cursor: last.and_then(|item| item.cursor_candidate.clone()),
        receipt: last.and_then(receipt_of),
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
    state: &FetchState,
    task: &strata_model::AccessTask,
    module: &Value,
    envelopes: &[RawEnvelope],
    pipeline: &Value,
    initial: bool,
) -> Result<Vec<Op>, Classified> {
    let rules =
        parse_pipeline(pipeline).map_err(|error| classified(error.class(), error.to_string()))?;
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
    let admission = Admission::from_value(module.get("admission").unwrap_or(&Value::Null))
        .map_err(|error| classified(error.class(), error.to_string()))?;
    let output = run(
        &Registry::builtins(),
        adapters::RunRequest {
            parts,
            pipeline: rules,
            reason: TaskReason::Schedule,
            source: source_view(task.source_id.as_str(), task.accessor.as_str())
                .map_err(|error| classified(error.class(), error.to_string()))?,
            resource: None,
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
    Ok(output.changeset.ops)
}

fn receipt_of(envelope: &RawEnvelope) -> Option<ReceiptBody> {
    let id = envelope.transport.receipt_id.as_ref()?.as_str().to_string();
    Some(ReceiptBody {
        id: id.clone(),
        service: "paid".into(),
        logical_key: id,
    })
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
