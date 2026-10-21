use ability_support::{AbilityCtx, Error, MethodId, Result};
use collector_ports::{CtxPersist, CtxQueue};
use collector_runtime::plans::{capacities, plans_of};
use planner::{Planner, Queue, canonical_source_id};
use serde_json::{Value, json};
use strata_model::Timestamp;
use strata_shared::text_field;

use crate::logic::snapshot::from_ctx;

pub async fn trigger(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let source = canonical_source_id(&text_field(&args, "source")?);
    let ctx = ctx.retain();
    let snapshot = from_ctx(ctx.as_ref()).await?;
    let module = snapshot.modules.get(&source);
    if module.is_none() || module.and_then(|m| m.get("kind")).and_then(Value::as_str) != Some("source") {
        return Err(Error::failed("信源未知"));
    }
    let plans = plans_of(&snapshot);
    let planner = Planner::new(
        CtxQueue::new(crate::ctx_call::wrap(ctx.clone())),
        CtxPersist::new(crate::ctx_call::wrap(ctx.clone()), "planner"),
    );
    let now = Timestamp::now();
    planner
        .make_due(&source, now)
        .await
        .map_err(|e| Error::failed(e.to_string()))?;
    planner
        .queue()
        .release_source_leases(&source)
        .await
        .map_err(|e| Error::failed(e.to_string()))?;
    let tasks = planner
        .tick(&plans, &capacities(), now)
        .await
        .map_err(|e| Error::failed(e.to_string()))?;
    let run_method = MethodId::new("run_task").map_err(|e| Error::failed(e.to_string()))?;
    let mut runs = Vec::new();
    for task in &tasks {
        let payload = serde_json::to_value(task).map_err(|e| Error::failed(e.to_string()))?;
        let outcome = ctx
            .call("runner", &run_method, json!({ "task": payload }))
            .await
            .map_err(|e| Error::failed(e.to_string()))?;
        runs.push(outcome.clone());
        if let Some(follows) = outcome.get("follows").and_then(Value::as_array) {
            for follow in follows {
                let _ = ctx
                    .call("runner", &run_method, json!({ "task": follow }))
                    .await;
            }
        }
    }
    let tasks_value = serde_json::to_value(&tasks).map_err(|e| Error::failed(e.to_string()))?;
    Ok(json!({ "tasks": tasks_value, "runs": runs }))
}
