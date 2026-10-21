use std::sync::Arc;

use ability_support::Result;
use serde_json::{Value, json};
use strata_client::Strata;
use strata_shared::failed;

pub async fn run(strata: &Arc<Strata>, _args: &Value) -> Result<Value> {
    let snapshot = strata.flush().await.map_err(|e| failed(e.to_string()))?;
    Ok(json!({ "snapshot": snapshot }))
}

pub async fn flush(ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    let _ = ctx;
    let args = strata_shared::normalize_args(args)?;
    run(&crate::runtime::strata()?, &args).await
}
