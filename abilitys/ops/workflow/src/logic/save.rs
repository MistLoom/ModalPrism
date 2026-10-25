//! 方法逻辑：save

use std::sync::Arc;

pub async fn run(
    engine: Arc<workflow::Engine>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    engine.store().save_workflow(&args).map_err(crate::fail)
}
