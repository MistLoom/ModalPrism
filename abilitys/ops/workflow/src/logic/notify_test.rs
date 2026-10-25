//! 方法逻辑：notify_test

use std::sync::Arc;

pub async fn run(
    engine: Arc<workflow::Engine>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    engine.notify_test(&args).await.map_err(crate::fail)
}
