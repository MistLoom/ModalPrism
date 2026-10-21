use ability_support::{AbilityCtx, MethodId, Result};
use serde_json::Value;

pub async fn run_task(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let method = MethodId::new("run_task").expect("method id");
    ctx.call("planner", &method, args).await
}
