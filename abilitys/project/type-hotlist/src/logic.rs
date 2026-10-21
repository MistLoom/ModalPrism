use ability_support::{AbilityCtx, Error, Result};
use serde_json::Value;
use strata_model::Rid;

pub async fn project(_ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let rid = args
        .get("rid")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::invalid_args("spec 无效"))?;
    let _ = Rid::parse(rid).map_err(|_| Error::invalid_args("spec 无效"))?;
    Ok(Value::Null)
}

pub async fn on_commit(_ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    if args.as_object().is_some_and(|object| !object.is_empty()) {
        return Err(Error::invalid_args("spec 无效"));
    }
    Ok(Value::Null)
}
