use ability_support::{AbilityCtx, Result};
use serde_json::Value;

pub async fn stop_listener(_ctx: &dyn AbilityCtx, _args: Value) -> Result<Value> {
    crate::runtime::stop_listener().await;
    Ok(Value::Null)
}
