use std::sync::Arc;

use ability_support::Result;
use serde::Deserialize;
use serde_json::{Value, json};
use strata_client::Strata;
use strata_shared::{failed, parse};

pub async fn run(strata: &Arc<Strata>, args: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        source: String,
        locator_key: String,
    }
    let args: Args = parse(args)?;
    let active = strata
        .task_active(&args.source, &args.locator_key)
        .await
        .map_err(|e| failed(e.to_string()))?;
    Ok(json!(active))
}
