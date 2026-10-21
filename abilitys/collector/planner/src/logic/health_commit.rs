use ability_support::{AbilityCtx, Error, Result};
use collector_ports::{CtxPersist, CtxQueue};
use planner::Planner;
use serde_json::Value;
use strata_model::Timestamp;

pub async fn health_commit(ctx: &dyn AbilityCtx, _args: Value) -> Result<Value> {
    let ctx = ctx.retain();
    let planner = Planner::new(
        CtxQueue::new(ctx.clone()),
        CtxPersist::new(ctx.clone(), "planner"),
    );
    planner
        .commit_snapshot(Timestamp::now())
        .await
        .map_err(|e| Error::failed(e.to_string()))?;
    Ok(serde_json::json!({}))
}
