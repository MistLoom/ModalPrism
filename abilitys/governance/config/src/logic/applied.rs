use ability_support::{AbilityCtx, Result};
use serde_json::{Value, json};

pub async fn applied(_ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let _ = args;
    let runtime = crate::runtime::get()?;
    let snap = runtime.store.applied();
    Ok(json!({
        "generation": snap.generation,
        "system": snap.system,
        "modules": snap.modules,
    }))
}
