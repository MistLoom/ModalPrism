use ability_support::{AbilityCtx, Error, Result};
use collector_gateway::policy_of;
use collector_gateway::SystemResolver;
use egress::{PrivateNetworks, Resolver};
use serde_json::{Value, json};

use crate::runtime;

pub async fn check(_ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let raw = args
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::invalid_args("spec 无效"))?;
    let url = url::Url::parse(raw).map_err(|_| Error::failed("gateway"))?;
    let host = url.host_str().ok_or_else(|| Error::failed("gateway"))?;
    let addresses = SystemResolver
        .lookup(host)
        .await
        .map_err(|e| Error::failed(e.message))?;
    let runtime = runtime::get()?;
    let policy = policy_of(runtime.system());
    let blocked = if policy.private_networks == PrivateNetworks::Allow {
        false
    } else {
        addresses.is_empty() || addresses.iter().all(|ip| egress::is_blocked(*ip))
    };
    let rendered: Vec<String> = addresses.iter().map(ToString::to_string).collect();
    Ok(json!({ "blocked": blocked, "addresses": rendered }))
}
