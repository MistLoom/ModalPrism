use ability_support::{AbilityCtx, Error, Result};
use collector_ports::{CtxPersist, CtxQueue};
use collector_runtime::plans::snapshot_interval;
use planner::Planner;
use serde_json::Value;
use strata_model::Timestamp;

use crate::logic::snapshot::from_ctx;

pub async fn health_snapshot(ctx: &dyn AbilityCtx, _args: Value) -> Result<Value> {
    let ctx = ctx.retain();
    let snapshot = from_ctx(ctx.as_ref()).await?;
    let planner = Planner::new(
        CtxQueue::new(crate::ctx_call::wrap(ctx.clone())),
        CtxPersist::new(crate::ctx_call::wrap(ctx.clone()), "planner"),
    );
    let now = Timestamp::now();
    let ops = planner
        .snapshot(now, snapshot_interval(&snapshot))
        .await
        .map_err(|e| Error::failed(e.to_string()))?;
    serde_json::to_value(ops).map_err(|e| Error::failed(e.to_string()))
}
