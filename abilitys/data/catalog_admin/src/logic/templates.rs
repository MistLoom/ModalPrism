//! 方法逻辑：templates

use std::sync::Arc;

pub async fn run(
    _ctx: Arc<dyn types::ReadCtx>,
    _args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    Ok(config::templates())
}
