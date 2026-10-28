//! 业务逻辑：blob_put

use ability_support::{AbilityCtx, Result};
use serde_json::Value;

pub async fn blob_put(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    super::proxy::forward(ctx, "blob_put", args).await
}
