use std::sync::Arc;
use std::time::Duration;

use ability_support::Result;
use serde::Deserialize;
use serde_json::Value;
use strata_client::Strata;
use strata_model::Timestamp;
use strata_shared::{failed, invalid_args, parse};

pub async fn run(strata: &Arc<Strata>, args: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        source: String,
        locator_key: String,
        lease_ms: u64,
        now: String,
    }
    let args: Args = parse(args)?;
    let now = Timestamp::parse(&args.now).map_err(|_| invalid_args("spec 无效"))?;
    let task = strata
        .lease_ready(
            &args.source,
            &args.locator_key,
            Duration::from_millis(args.lease_ms),
            now,
        )
        .await
        .map_err(|e| failed(e.to_string()))?;
    match task {
        Some(task) => serde_json::to_value(task).map_err(|e| failed(e.to_string())),
        None => Ok(Value::Null),
    }
}
