//! 方法逻辑：changes

use std::sync::Arc;

pub async fn run(
    ctx: Arc<dyn types::ReadCtx>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    crate::map(types::read::call(ctx.as_ref(), "strata_read.changes", args).await)
}
