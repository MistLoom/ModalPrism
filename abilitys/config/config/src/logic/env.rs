//! 业务逻辑：env

use ability_support::{AbilityCtx, Error, Result};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::format::ConfigFormat;
use crate::load::{load_process_env, load_str};

#[derive(Deserialize)]
struct EnvArgs {
    prefix: Option<String>,
    content: Option<String>,
}

pub async fn env(_ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let a: EnvArgs =
        serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
    let has_content = a.content.is_some();
    let mut data = if let Some(content) = a.content {
        load_str(&content, ConfigFormat::Env)?
    } else {
        load_process_env(a.prefix.as_deref())
    };
    if has_content {
        if let Some(prefix) = a.prefix.as_deref() {
            if let Value::Object(map) = data {
                let filtered: serde_json::Map<String, Value> = map
                    .into_iter()
                    .filter_map(|(k, v)| k.strip_prefix(prefix).map(|rest| (rest.to_string(), v)))
                    .collect();
                data = Value::Object(filtered);
            }
        }
    }
    Ok(json!({ "data": data }))
}
