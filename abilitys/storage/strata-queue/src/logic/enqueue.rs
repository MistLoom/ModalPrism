use std::sync::Arc;

use ability_support::Result;
use serde::Deserialize;
use serde_json::{Value, json};
use strata_client::Strata;
use strata_model::AccessTask;
use strata_shared::{failed, parse};

pub async fn run(strata: &Arc<Strata>, args: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        tasks: Vec<AccessTask>,
    }
    let args: Args = parse(args)?;
    strata
        .enqueue(&args.tasks)
        .await
        .map_err(|e| failed(e.to_string()))?;
    Ok(json!({}))
}
