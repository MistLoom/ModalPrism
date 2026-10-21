use ability_support::{AbilityCtx, Error, Result};
use serde_json::Value;
use strata_shared::text_field;

use crate::logic::governance;
use crate::runtime;

pub async fn reset_circuit(_ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let host = text_field(&args, "host")?;
    let confirm = args
        .get("confirm")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !confirm {
        return Err(Error::invalid_args("需要 confirm: true"));
    }
    let runtime = runtime::get()?;
    let gw = runtime.gateway();
    governance::reset_circuit(&gw, &host).await
}
