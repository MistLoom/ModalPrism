//! 内置访问器 Ability（组合根内联 Provider）。

use std::sync::Arc;
use std::time::Duration;

use ability_support::{AbilityProvider, AbilitySpec, Result};
use collector_boot::BootResources;
use collector_gateway::{call_policy_from, policy_of};
use collector_runtime::execute;
use serde_json::Value;

pub fn provider(
    services: Arc<BootResources>,
    instance: &'static str,
    builtin: &'static str,
) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must(instance);
    spec = spec.on(
        "validate",
        "校验访问规格",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            let builtin = builtin.to_string();
            move |_ctx, args| {
                let services = Arc::clone(&services);
                let builtin = builtin.clone();
                ability_support::box_fut(async move { validate(&services, &builtin, args) })
            }
        },
    );
    spec = spec.on(
        "access",
        "执行一次访问",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            let builtin = builtin.to_string();
            move |_ctx, args| {
                let services = Arc::clone(&services);
                let builtin = builtin.clone();
                ability_support::box_fut(async move { access(&services, &builtin, args).await })
            }
        },
    );
    spec.into_provider()
}

fn validate(services: &BootResources, builtin: &str, args: Value) -> Result<Value> {
    let spec = args
        .get("spec")
        .ok_or_else(|| ability_support::Error::invalid_args("spec 无效"))?;
    let implementation = format!("builtin:{builtin}");
    let accessor = execute::open_accessor(
        &implementation,
        &services.browser_program(),
        &services.proxy_url(),
    )
    .map_err(|error| ability_support::Error::invalid_args(error.message))?;
    accessor
        .validate(spec)
        .map_err(|error| ability_support::Error::invalid_args(error.message))?;
    Ok(serde_json::json!({}))
}

async fn access(services: &BootResources, builtin: &str, args: Value) -> Result<Value> {
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
        .map_err(|_| ability_support::Error::invalid_args("spec 无效"))?;
    let implementation = format!("builtin:{builtin}");
    let accessor = execute::open_accessor(
        &implementation,
        &services.browser_program(),
        &services.proxy_url(),
    )
    .map_err(|error| ability_support::Error::invalid_args(error.message))?;
    let policy_value = args
        .policy
        .clone()
        .unwrap_or_else(|| Value::Object(Default::default()));
    let request = accessors::AccessRequest {
        task_id: strata_model::TaskId::new(args.task_id)
            .map_err(|_| ability_support::Error::invalid_args("spec 无效"))?,
        source_id: strata_model::SourceId::new(args.source_id)
            .map_err(|_| ability_support::Error::invalid_args("spec 无效"))?,
        locator: args.locator,
        spec: args.spec,
        cursor: args.cursor,
        hints: accessors::AccessHints::default(),
        policy: Arc::new(policy_value.clone()),
        deadline: std::time::Instant::now() + Duration::from_secs(30),
    };
    let snap = services.config().applied();
    let egress_policy = policy_of(&snap.system);
    let paid_policy = call_policy_from(&snap.system, &policy_value);
    let envelopes = accessor
        .access(request, &services.access_context(egress_policy, paid_policy))
        .await
        .map_err(|error| ability_support::Error::failed(error.message))?;
    serde_json::to_value(envelopes).map_err(|source| ability_support::Error::failed(source.to_string()))
}
