//! 方法逻辑：tick

use std::sync::Arc;

pub async fn run(
    engine: Arc<workflow::Engine>,
    _args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let tick = engine.tick().await.map_err(crate::fail)?;
    Ok(tick.value())
}
