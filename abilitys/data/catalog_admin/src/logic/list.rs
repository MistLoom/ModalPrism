//! 方法逻辑：list

use std::sync::Arc;

pub async fn run(
    ctx: Arc<dyn types::ReadCtx>,
    _args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    config::list(ctx.config_dir()).map_err(crate::fail)
}
