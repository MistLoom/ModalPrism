use ability_support::{AbilityCtx, Error, Result};
use collector_ports::{CtxPersist, CtxQueue};
use planner::Planner;
use serde_json::{Value, json};
use strata_shared::text_field;

pub async fn source_status(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let source = text_field(&args, "source")?;
    let ctx = ctx.retain();
    let planner = Planner::new(
        CtxQueue::new(ctx.clone()),
        CtxPersist::new(ctx.clone(), "planner"),
    );
    let status = planner
        .source_status(&source)
        .await
        .map_err(|e| Error::failed(e.to_string()))?;
    Ok(json!({
        "key": planner::health_source_subject(&source),
        "status": status.health_status,
        "alert": status.alert,
        "lastError": status.last_error,
        "lastYieldAt": status.last_yield_at,
    }))
}
