use std::sync::Arc;
use std::time::Duration;

use ability_support::Result;
use serde::Deserialize;
use serde_json::Value;
use strata_client::Strata;
use strata_model::{QueueName, Timestamp};
use strata_shared::{failed, invalid_args, parse};

pub async fn run(strata: &Arc<Strata>, args: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        queue: QueueName,
        count: u64,
        lease_ms: u64,
        now: String,
    }
    let args: Args = parse(args)?;
    let count = usize::try_from(args.count).map_err(|_| invalid_args("spec 无效"))?;
    let now = Timestamp::parse(&args.now).map_err(|_| invalid_args("spec 无效"))?;
    let tasks = strata
        .lease_follows(
            args.queue,
            count,
            Duration::from_millis(args.lease_ms),
            now,
        )
        .await
        .map_err(|e| failed(e.to_string()))?;
    serde_json::to_value(tasks).map_err(|e| failed(e.to_string()))
}
