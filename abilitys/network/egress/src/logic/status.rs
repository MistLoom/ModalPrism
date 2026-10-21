use ability_support::{AbilityCtx, Result};
use serde_json::{Value, json};

use crate::runtime;

pub async fn status(_ctx: &dyn AbilityCtx, _args: Value) -> Result<Value> {
    let runtime = runtime::get()?;
    Ok(json!({ "proxy": runtime.proxy_url() }))
}
