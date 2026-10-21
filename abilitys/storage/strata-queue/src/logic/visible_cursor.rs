use std::sync::Arc;

use ability_support::Result;
use serde::Deserialize;
use serde_json::Value;
use strata_client::Strata;
use strata_shared::{failed, parse};

pub async fn run(strata: &Arc<Strata>, args: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        source: String,
    }
    let args: Args = parse(args)?;
    let state = strata
        .state("cursor", &args.source)
        .await
        .map_err(|e| failed(e.to_string()))?;
    Ok(state.map(|s| s.value).unwrap_or(Value::Null))
}
