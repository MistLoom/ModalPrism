//! 业务逻辑：blob_get

use ability_support::{AbilityCtx, Result};
use serde_json::Value;

pub async fn blob_get(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    super::proxy::forward(ctx, "blob_get", args).await
}
