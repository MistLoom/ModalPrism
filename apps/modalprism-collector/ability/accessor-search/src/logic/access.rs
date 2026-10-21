use ability_support::{AbilityCtx, Result};
use serde_json::Value;

const BUILTIN: &str = "search";

pub async fn access(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    collector_accessor_shared::access(ctx, &crate::runtime::bind()?, BUILTIN, args).await
}
