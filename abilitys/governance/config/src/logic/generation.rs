use ability_support::{AbilityCtx, Result};
use serde_json::{Value, json};

pub async fn generation(_ctx: &dyn AbilityCtx, _args: Value) -> Result<Value> {
    let runtime = crate::runtime::get()?;
    Ok(json!({ "generation": runtime.store.snapshot().generation }))
}
