use std::sync::Arc;

use ability_support::Result;
use serde_json::Value;
use strata_client::Strata;
use strata_shared::failed;

pub async fn run(strata: &Arc<Strata>, _args: &Value) -> Result<Value> {
    let tasks = strata
        .release_leases()
        .await
        .map_err(|e| failed(e.to_string()))?;
    serde_json::to_value(tasks).map_err(|e| failed(e.to_string()))
}

pub async fn release_leases(ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    let _ = ctx;
    let args = strata_shared::normalize_args(args)?;
    run(&crate::runtime::strata()?, &args).await
}
