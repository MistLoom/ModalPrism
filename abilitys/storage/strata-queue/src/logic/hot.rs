use std::sync::Arc;

use ability_support::Result;
use serde::Deserialize;
use serde_json::{Value, json};
use strata_client::Strata;
use strata_shared::{failed, invalid_args, parse};

pub async fn load(strata: &Arc<Strata>, args: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        kind: String,
    }
    let args: Args = parse(args)?;
    match strata
        .hot_get(&args.kind, "state")
        .await
        .map_err(|e| failed(e.to_string()))?
    {
        Some(value) => Ok(value),
        None => Ok(Value::Null),
    }
}

pub async fn store(strata: &Arc<Strata>, args: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        kind: String,
        value: Value,
    }
    let args: Args = parse(args)?;
    strata
        .hot_put(&args.kind, "state", &args.value)
        .await
        .map_err(|e| failed(e.to_string()))?;
    Ok(json!({}))
}
