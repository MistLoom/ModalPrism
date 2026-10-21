use ability_support::{AbilityCtx, MethodId, Result};
use bytes::Bytes;
use collector_gateway::{EGRESS_POLICY, policy_from, policy_of};
use serde::Deserialize;
use serde_json::{Value, json};
use strata_shared::failed;

use crate::runtime;

pub async fn http_request(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let runtime = runtime::get()?;
    let applied = ctx
        .call(
            "config",
            &MethodId::new("applied").expect("method id"),
            json!({}),
        )
        .await?;
    let system = applied
        .get("system")
        .cloned()
        .unwrap_or_else(|| runtime.system().clone());
    run(&runtime.gateway(), &system, policy_of(&system), &args).await
}

pub async fn run(
    gateway: &crate::runtime::EgressGw,
    system: &Value,
    default_policy: egress::Policy,
    args: &Value,
) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        method: String,
        url: String,
        headers: Vec<(String, String)>,
        body: serde_json::Value,
        #[serde(default)]
        module_policy: Option<Value>,
    }
    let args: Args = strata_shared::parse(args)?;
    let policy = args
        .module_policy
        .as_ref()
        .map(|module| policy_from(system, module))
        .or_else(|| EGRESS_POLICY.try_with(|p| p.clone()).ok())
        .unwrap_or(default_policy);
    let body = match args.body {
        Value::String(text) => text.into_bytes(),
        Value::Null => Vec::new(),
        other => serde_json::to_vec(&other).unwrap_or_default(),
    };
    let response = EGRESS_POLICY
        .scope(policy.clone(), async {
            gateway
                .request_governed(
                    &policy,
                    accessors::HttpRequest {
                        method: args.method,
                        url: args.url,
                        headers: args.headers,
                        body: bytes::Bytes::from(body),
                    },
                )
                .await
        })
        .await
        .map_err(|e| failed(e.message))?;
    Ok(json!({
        "status": response.status,
        "headers": response.headers,
        "body": response.body.to_vec(),
        "finalUrl": response.final_url,
    }))
}
