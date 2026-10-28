//! 业务逻辑：run

use std::path::Path;

use ability_support::{AbilityCtx, Error, Result};
use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct Args {
    path: String,
    #[serde(default)]
    contents: Option<String>,
    #[serde(default)]
    bytes_base64: Option<String>,
    #[serde(default)]
    overwrite: Option<bool>,
}

pub async fn run(_ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let a: Args = serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
    let path = Path::new(&a.path);
    let overwrite = a.overwrite.unwrap_or(true);
    if !overwrite && path.exists() {
        return Err(Error::failed(format!(
            "write_file: 目标已存在且 overwrite=false: {}",
            a.path
        )));
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| {
                Error::io(format!("write_file create_dir {}: {e}", parent.display()))
            })?;
        }
    }
    let bytes = if let Some(b64) = &a.bytes_base64 {
        base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .map_err(|e| Error::invalid_args(format!("bytes_base64 无效: {e}")))?
    } else if let Some(text) = &a.contents {
        text.as_bytes().to_vec()
    } else {
        return Err(Error::invalid_args("需要 contents 或 bytes_base64"));
    };
    std::fs::write(path, &bytes).map_err(|e| Error::io(format!("write_file {}: {e}", a.path)))?;
    Ok(json!({ "path": a.path, "bytes": bytes.len() }))
}
