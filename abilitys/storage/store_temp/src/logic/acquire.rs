//! 业务逻辑：acquire

use ability_support::{AbilityCtx, Result};
use serde_json::Value;

pub async fn acquire(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    super::proxy::forward(ctx, "acquire", args).await
}
