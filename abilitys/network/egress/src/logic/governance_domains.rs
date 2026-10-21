use ability_support::{AbilityCtx, Result};
use serde_json::Value;

use crate::logic::governance;
use crate::runtime;

pub async fn governance_domains(_ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let runtime = runtime::get()?;
    let gw = runtime.gateway();
    governance::domains(&gw, runtime.proxy_url()).await
}
