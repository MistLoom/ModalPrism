//! 内置访问器 validate/access，经 AbilityCtx 调用 egress 与 paid_gateway。

use std::sync::Arc;
use std::time::Duration;

use ability_support::{AbilityCtx, Error, Result};
use collector_bind::BindConfig;
use collector_gateway::{call_policy_from, policy_of};
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
        deadline: std::time::Instant::now() + Duration::from_secs(30),
    };
    let egress_policy = policy_of(&bind.system);
    let paid_policy = call_policy_from(&bind.system, &policy_value);
    let access = ability_collector_runner::access::AbilityAccess::build(
        ctx.retain(),
        egress_policy,
        paid_policy,
        bind.data_dir.join("staging"),
    );
    let envelopes = accessor
        .access(request, &access)
        .await
        .map_err(|error| Error::failed(error.message))?;
    serde_json::to_value(envelopes).map_err(|source| Error::failed(source.to_string()))
}
