//! 业务逻辑：load

use ability_support::{AbilityCtx, Error, Result};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::format::{ConfigFormat, resolve_path};
use crate::load::load_file;

#[derive(Deserialize)]
struct LoadArgs {
    path: String,
    #[serde(default)]
    format: Option<String>,
}

pub async fn load(_ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let a: LoadArgs =
        serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
    let format = match a.format.as_deref() {
        None | Some("") => ConfigFormat::Auto,
        Some(s) => s.parse()?,
    };
    let path = resolve_path(&a.path)?;
    let data = load_file(&path, format)?;
    Ok(json!({ "path": path.display().to_string(), "data": data }))
}
