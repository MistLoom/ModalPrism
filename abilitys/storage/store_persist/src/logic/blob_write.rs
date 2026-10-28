//! 业务逻辑：blob_write

use ability_support::{AbilityCtx, Result};
use serde_json::Value;

pub async fn blob_write(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    super::proxy::forward(ctx, "blob_write", args).await
}
