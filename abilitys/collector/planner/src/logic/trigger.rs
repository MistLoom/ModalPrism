use ability_support::{AbilityCtx, Error, Result};
use planner::Planner;
use serde_json::Value;
use strata_model::Timestamp;
use strata_shared::{persist::{Hot, StrataQueue}, text_field};

pub async fn run(
    ctx: &dyn AbilityCtx,
    planner: &Planner<StrataQueue, Hot>,
    args: &Value,
) -> Result<Value> {
    let source = text_field(args, "source")?;
    planner
        .make_due(&source, Timestamp::now())
        .await
        .map_err(|e| Error::failed(e.to_string()))?;
    crate::logic::tick::run(ctx, planner).await
}
