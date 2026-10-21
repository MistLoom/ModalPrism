use ability_support::{AbilityCtx, Error, Result};
use collector_ports::{CtxPersist, CtxQueue};
use planner::Planner;
use serde_json::Value;
use strata_model::Timestamp;
use strata_shared::text_field;

pub async fn trigger(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let source = text_field(&args, "source")?;
    let ctx = ctx.retain();
    let planner = Planner::new(
        CtxQueue::new(ctx.clone()),
        CtxPersist::new(ctx.clone(), "planner"),
    );
    planner
        .make_due(&source, Timestamp::now())
        .await
        .map_err(|e| Error::failed(e.to_string()))?;
    crate::logic::tick::tick(ctx.as_ref(), Value::Object(Default::default())).await
}
