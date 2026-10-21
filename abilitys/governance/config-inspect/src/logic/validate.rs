use ability_support::{AbilityCtx, Result};
use serde_json::{Value, json};

pub async fn validate(_ctx: &dyn AbilityCtx, _args: Value) -> Result<Value> {
    let runtime = crate::runtime::get()?;
    config::validate(&runtime.config_dir, &runtime.load_options)
        .map_err(|e| ability_support::Error::failed(e.to_string()))?;
    Ok(json!({ "ok": true }))
}
