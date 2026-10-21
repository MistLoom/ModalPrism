use ability_support::{AbilityCtx, Result};
use serde_json::{Value, json};

pub async fn layout(_ctx: &dyn AbilityCtx, _args: Value) -> Result<Value> {
    let runtime = crate::runtime::get()?;
    Ok(json!({
        "dataDir": runtime.bind.data_dir,
        "configDir": runtime.bind.config_dir,
    }))
}
