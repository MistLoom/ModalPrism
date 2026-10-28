//! 业务逻辑：delete

use ability_support::{AbilityCtx, Result};
use serde_json::Value;

pub async fn delete(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    super::proxy::forward(ctx, "delete", args).await
}
