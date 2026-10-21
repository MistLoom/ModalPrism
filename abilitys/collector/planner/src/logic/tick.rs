use ability_support::{AbilityCtx, Error, Result};
use collector_runtime::plans::{capacities, plans_of};
use planner::Planner;
use serde_json::Value;
use strata_model::Timestamp;
use strata_shared::persist::{Hot, StrataQueue};

use crate::logic::snapshot::from_ctx;

pub async fn run(
    ctx: &dyn AbilityCtx,
    planner: &Planner<StrataQueue, Hot>,
) -> Result<Value> {
    let snapshot = from_ctx(ctx).await?;
    let plans = plans_of(&snapshot);
    let tasks = planner
        .tick(&plans, &capacities(), Timestamp::now())
        .await
        .map_err(|e| Error::failed(e.to_string()))?;
    serde_json::to_value(tasks).map_err(|e| Error::failed(e.to_string()))
}
