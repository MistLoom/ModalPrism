use ability_support::{AbilityCtx, Error, Result};
use serde_json::Value;

pub async fn reserve(_ctx: &dyn AbilityCtx, _args: Value) -> Result<Value> {
    Err(Error::failed("reserve"))
}
