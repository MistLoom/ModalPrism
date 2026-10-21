use ability_support::{AbilityCtx, Result};
use serde_json::{Value, json};

pub async fn load(_ctx: &dyn AbilityCtx, _args: Value) -> Result<Value> {
    let runtime = crate::runtime::get()?;
    runtime
        .store
        .reload()
        .map_err(|e| ability_support::Error::failed(e.to_string()))?;
    Ok(json!({ "generation": runtime.store.snapshot().generation }))
}
