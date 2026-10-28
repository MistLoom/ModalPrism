//! 业务逻辑：get

use ability_support::{AbilityCtx, Result};
use serde_json::Value;

pub async fn get(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    super::proxy::forward(ctx, "get", args).await
}
