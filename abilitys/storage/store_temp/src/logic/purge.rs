//! 业务逻辑：purge

use ability_support::{AbilityCtx, Result};
use serde_json::Value;

pub async fn purge(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    super::proxy::forward(ctx, "purge", args).await
}
