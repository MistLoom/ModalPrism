use ability_support::{AbilityCtx, Error, Result};
use collector_ports::{CtxPersist, CtxQueue};
use planner::{Planner, canonical_source_id};
use serde_json::{Value, json};
use strata_shared::text_field;

use crate::logic::snapshot::from_ctx;

pub async fn source_status(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let source = canonical_source_id(&text_field(&args, "source")?);
    let ctx = ctx.retain();
    let snapshot = from_ctx(ctx.as_ref()).await?;
    let module = snapshot.modules.get(&source);
    if module.is_none() || module.and_then(|m| m.get("kind")).and_then(Value::as_str) != Some("source") {
        return Err(Error::failed("信源未知"));
    }
    let planner = Planner::new(
        CtxQueue::new(crate::ctx_call::wrap(ctx.clone())),
        CtxPersist::new(crate::ctx_call::wrap(ctx.clone()), "planner"),
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
