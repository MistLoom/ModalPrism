//! 业务逻辑：release

use ability_support::{AbilityCtx, Result};
use serde_json::Value;

pub async fn release(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    super::proxy::forward(ctx, "release", args).await
}
