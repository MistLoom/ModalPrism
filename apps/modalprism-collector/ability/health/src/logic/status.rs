use ability_support::{AbilityCtx, Error, MethodId, Result};
use serde_json::Value;

pub async fn status(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    ctx.call(
        "planner",
        &MethodId::new("source_status").unwrap(),
        args,
    )
    .await
    .map_err(|e| Error::failed(e.to_string()))
}
