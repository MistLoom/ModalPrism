use ability_support::{AbilityCtx, Error, Result};
use collector_ports::{CtxPersist, CtxQueue};
use planner::Planner;
use serde_json::Value;
use strata_model::Timestamp;

pub async fn shutdown(ctx: &dyn AbilityCtx, _args: Value) -> Result<Value> {
    let ctx = ctx.retain();
    let planner = Planner::new(
        CtxQueue::new(ctx.clone()),
        CtxPersist::new(ctx.clone(), "planner"),
    );
    planner
        .shutdown(Timestamp::now())
        .await
        .map_err(|e| Error::failed(e.to_string()))?;
    Ok(serde_json::json!({}))
}
