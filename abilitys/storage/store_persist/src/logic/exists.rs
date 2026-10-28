//! 业务逻辑：exists

use ability_support::{AbilityCtx, Result};
use serde_json::Value;

pub async fn exists(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    super::proxy::forward(ctx, "exists", args).await
}
