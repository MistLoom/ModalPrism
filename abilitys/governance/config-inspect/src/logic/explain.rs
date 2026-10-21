use ability_support::{AbilityCtx, Error, Result};
use serde_json::{Value, json};

pub async fn explain(_ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let runtime = crate::runtime::get()?;
    let module = args
        .get("module")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::invalid_args("spec 无效"))?;
    let fields = config::explain(&runtime.store.snapshot(), module)
        .map_err(|e| Error::failed(e.to_string()))?;
    let items = fields
        .into_iter()
        .map(|field| {
            json!({
                "path": field.path,
                "value": field.value,
                "file": field.file,
                "line": field.line,
                "presetId": field.preset_id,
                "viaPreset": field.via_preset,
                "systemScope": field.system_scope,
            })
        })
        .collect();
    Ok(Value::Array(items))
}
