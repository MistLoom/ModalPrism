use ability_support::{AbilityCtx, Error, Result};
use collector_ports::{CtxPersist, CtxQueue};
use collector_runtime::plans::{capacities, plans_of};
use planner::Planner;
use serde_json::Value;
use strata_model::Timestamp;

use crate::logic::snapshot::from_ctx;

pub async fn tick(ctx: &dyn AbilityCtx, _args: Value) -> Result<Value> {
    let ctx = ctx.retain();
    let snapshot = from_ctx(ctx.as_ref()).await?;
    let plans = plans_of(&snapshot);
    let planner = Planner::new(
        CtxQueue::new(crate::ctx_call::wrap(ctx.clone())),
        CtxPersist::new(crate::ctx_call::wrap(ctx.clone()), "planner"),
    );
    let tasks = planner
        .tick(&plans, &capacities(), Timestamp::now())
        .await
        .map_err(|e| Error::failed(e.to_string()))?;
    serde_json::to_value(tasks).map_err(|e| Error::failed(e.to_string()))
}
