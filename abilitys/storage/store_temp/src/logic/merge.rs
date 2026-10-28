//! 业务逻辑：merge

use ability_support::{AbilityCtx, Result};
use serde_json::Value;

pub async fn merge(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    super::proxy::forward(ctx, "merge", args).await
}
