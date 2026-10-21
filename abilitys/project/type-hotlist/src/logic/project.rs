use ability_support::{AbilityCtx, Error, Result};
use serde_json::Value;
use strata_model::Rid;

pub async fn project(_ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let _strata = crate::runtime::strata()?;
    let rid = args
        .get("rid")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::invalid_args("spec 无效"))?;
    let _ = Rid::parse(rid).map_err(|_| Error::invalid_args("spec 无效"))?;
    Ok(Value::Null)
}
