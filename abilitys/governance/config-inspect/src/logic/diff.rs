use ability_support::{AbilityCtx, Result};
use serde_json::{Value, json};

pub async fn diff(_ctx: &dyn AbilityCtx, _args: Value) -> Result<Value> {
    let runtime = crate::runtime::get()?;
    let fresh = config::validate(&runtime.config_dir, &runtime.load_options)
        .map_err(|e| ability_support::Error::failed(e.to_string()))?;
    let changes = config::diff(&runtime.store.snapshot(), &fresh);
    let items = changes
        .into_iter()
        .map(|change| {
            json!({
                "path": change.path,
                "before": change.before,
                "after": change.after,
            })
        })
        .collect();
    Ok(Value::Array(items))
}
