//! 方法逻辑：project

use std::sync::Arc;

use types::ReadCtx;

pub async fn run(
    ctx: Arc<dyn ReadCtx>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    crate::map(types::read::call(ctx.as_ref(), "type_hotlist.project", args).await)
}
