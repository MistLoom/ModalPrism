//! 内置访问器 validate/access，经 AbilityCtx 调用 egress 与 paid_gateway。

mod access;
mod ctx_call;

pub use access::AbilityAccess;
pub use ctx_call::wrap as ctx_call;

use std::sync::Arc;
use std::time::Duration;

use ability_support::{AbilityCtx, Error, MethodId, Result};
use collector_bind::BindConfig;
use collector_gateway::{call_policy_from, policy_from};
use collector_runtime::execute;
use serde_json::Value;

pub async fn validate(bind: &BindConfig, builtin: &str, args: Value) -> Result<Value> {
    let spec = args
        .get("spec")
        .ok_or_else(|| Error::invalid_args("spec 无效"))?;
    let implementation = format!("builtin:{builtin}");
    let accessor = execute::open_accessor(
        &implementation,
        bind
            .browser_program
            .as_deref()
            .unwrap_or_else(|| std::path::Path::new("moli")),
        bind.proxy_url.as_deref().unwrap_or(""),
    )
    .map_err(|error| Error::invalid_args(error.message))?;
    accessor
        .validate(spec)
        .map_err(|error| Error::invalid_args(error.message))?;
    Ok(serde_json::json!({}))
}

pub async fn access(
    ctx: &dyn AbilityCtx,
    bind: &BindConfig,
    builtin: &str,
    args: Value,
) -> Result<Value> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        task_id: String,
        source_id: String,
        locator: strata_model::Locator,
        spec: Value,
        #[serde(default)]
        cursor: Option<Value>,
        #[serde(default)]
        policy: Option<Value>,
    }
    let args: Args = serde_json::from_value(args)
        .map_err(|_| Error::invalid_args("spec 无效"))?;
    let implementation = format!("builtin:{builtin}");
    let accessor = execute::open_accessor(
        &implementation,
        bind
            .browser_program
            .as_deref()
            .unwrap_or_else(|| std::path::Path::new("moli")),
        bind.proxy_url.as_deref().unwrap_or(""),
    )
    .map_err(|error| Error::invalid_args(error.message))?;
    let policy_value = args
        .policy
        .clone()
        .unwrap_or_else(|| Value::Object(Default::default()));
    let applied = ctx
        .call(
            "config",
            &MethodId::new("applied").expect("method id"),
            serde_json::json!({}),
        )
        .await?;
    let system = applied.get("system").cloned().unwrap_or_else(|| bind.system.clone());
    let deadline = access_deadline(&system, &policy_value);
    let request = accessors::AccessRequest {
        task_id: strata_model::TaskId::new(args.task_id)
            .map_err(|_| Error::invalid_args("spec 无效"))?,
        source_id: strata_model::SourceId::new(args.source_id)
            .map_err(|_| Error::invalid_args("spec 无效"))?,
        locator: args.locator,
        spec: args.spec,
        cursor: args.cursor,
        hints: accessors::AccessHints::default(),
        policy: Arc::new(policy_value.clone()),
        deadline: std::time::Instant::now() + deadline,
    };
    let egress_policy = policy_from(&system, &policy_value);
    let paid_policy = call_policy_from(&system, &policy_value);
    let access = AbilityAccess::build(
        ctx.retain(),
        egress_policy,
        policy_value,
        paid_policy,
        bind.data_dir.join("staging"),
    );
    let envelopes = accessor
        .access(request, &access)
        .await
        .map_err(|error| Error::failed(error.message))?;
    serde_json::to_value(envelopes).map_err(|source| Error::failed(source.to_string()))
}

fn access_deadline(system: &Value, policy: &Value) -> Duration {
    let mut secs = 30u64;
    if let Some(text) = system.pointer("/limits/timeout").and_then(Value::as_str) {
        if let Some(duration) = parse_duration(text) {
            secs = duration.as_secs().max(1);
        }
    }
    if let Some(text) = policy.get("timeout").and_then(Value::as_str) {
        if let Some(duration) = parse_duration(text) {
            secs = duration.as_secs().max(1);
        }
    }
    Duration::from_secs(secs)
}

fn parse_duration(text: &str) -> Option<Duration> {
    let split = text.find(|ch: char| !ch.is_ascii_digit())?;
    let count: u64 = text[..split].parse().ok()?;
    let unit = &text[split..];
    let secs = match unit {
        "s" => count,
        "m" => count * 60,
        "h" => count * 3600,
        _ => return None,
    };
    Some(Duration::from_secs(secs))
}
