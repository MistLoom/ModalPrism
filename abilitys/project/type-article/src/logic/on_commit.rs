use ability_support::{AbilityCtx, Error, Result};
use serde_json::Value;

pub async fn on_commit(_ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    if args.as_object().is_some_and(|o| !o.is_empty()) {
        return Err(Error::invalid_args("spec 无效"));
    }
    Ok(Value::Null)
}
