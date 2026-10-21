use ability_support::{AbilityCtx, Error, Result};
use collector_ports::{CtxPersist, CtxQueue};
use planner::Planner;
use serde_json::{Value, json};
use strata_shared::text_field;

pub async fn next(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let source = text_field(&args, "source")?;
    let ctx = ctx.retain();
    let planner = Planner::new(
        CtxQueue::new(crate::ctx_call::wrap(ctx.clone())),
        CtxPersist::new(crate::ctx_call::wrap(ctx.clone()), "planner"),
    );
    let status = planner
        .source_status(&source)
        .await
        .map_err(|e| Error::failed(e.to_string()))?;
    Ok(json!({
        "nextAt": status.next_at,
        "intervalMs": status.interval_ms,
        "healthStatus": status.health_status,
        "alert": status.alert,
        "skipReason": status.skip_reason,
        "budgetUsed": status.budget_used,
        "lastYieldAt": status.last_yield_at,
        "todayYieldCount": status.today_yield_count,
        "lastError": status.last_error,
    }))
}
