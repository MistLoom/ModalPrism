use std::sync::Arc;

use ability_support::Result;
use serde::Deserialize;
use serde_json::Value;
use strata_client::Strata;
use strata_model::{ApplyOpts, ChangeSet};
use strata_shared::{failed, parse};

pub async fn run(strata: &Arc<Strata>, args: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        changeset: ChangeSet,
    }
    let args: Args = parse(args)?;
    let result = strata
        .apply(args.changeset, ApplyOpts::default())
        .await
        .map_err(|e| failed(e.to_string()))?;
    serde_json::to_value(result).map_err(|e| failed(e.to_string()))
}

pub async fn apply(ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    let _ = ctx;
    let args = strata_shared::normalize_args(args)?;
    run(&crate::runtime::strata()?, &args).await
}
