//! 业务逻辑：stat

use ability_support::{AbilityCtx, Result};
use serde_json::Value;

pub async fn stat(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    super::proxy::forward(ctx, "stat", args).await
}
