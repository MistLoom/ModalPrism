//! 业务逻辑：list

use ability_support::{AbilityCtx, Result};
use serde_json::Value;

pub async fn list(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    super::proxy::forward(ctx, "list", args).await
}
