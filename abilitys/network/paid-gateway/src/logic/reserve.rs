use ability_support::{AbilityCtx, Result};
use serde_json::{Value, json};

pub async fn reserve(_ctx: &dyn AbilityCtx, _args: Value) -> Result<Value> {
    Ok(json!({ "ok": true }))
}
