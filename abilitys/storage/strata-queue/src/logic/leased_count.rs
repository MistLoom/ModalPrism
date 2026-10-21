use std::sync::Arc;

use ability_support::Result;
use serde::Deserialize;
use serde_json::{Value, json};
use strata_client::Strata;
use strata_model::{QueueName, Timestamp};
use strata_shared::{failed, invalid_args, parse};

pub async fn run(strata: &Arc<Strata>, args: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        queue: QueueName,
        now: String,
    }
    let args: Args = parse(args)?;
    let now = Timestamp::parse(&args.now).map_err(|_| invalid_args("spec 无效"))?;
    let count = strata
        .leased_count(args.queue, now)
        .await
        .map_err(|e| failed(e.to_string()))?;
    Ok(json!(count))
}

pub async fn leased_count(ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    let _ = ctx;
    let args = strata_shared::normalize_args(args)?;
    run(&crate::runtime::strata()?, &args).await
}
