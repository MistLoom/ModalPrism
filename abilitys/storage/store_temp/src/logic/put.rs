//! 业务逻辑：put

use ability_support::{AbilityCtx, Result};
use serde_json::Value;

pub async fn put(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    super::proxy::forward(ctx, "put", args).await
}
