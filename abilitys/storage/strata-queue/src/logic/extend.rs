use std::sync::Arc;
use std::time::Duration;

use ability_support::Result;
use serde::Deserialize;
use serde_json::{Value, json};
use strata_client::Strata;
use strata_model::TaskId;
use strata_shared::{failed, invalid_args, parse};

pub async fn run(strata: &Arc<Strata>, args: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        task_id: String,
        lease_ms: u64,
    }
    let args: Args = parse(args)?;
    let task_id = TaskId::new(args.task_id).map_err(|_| invalid_args("spec 无效"))?;
    strata
        .extend(&task_id, Duration::from_millis(args.lease_ms))
        .await
        .map_err(|e| failed(e.to_string()))?;
    Ok(json!({}))
}
