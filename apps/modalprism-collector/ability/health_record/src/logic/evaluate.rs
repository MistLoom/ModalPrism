//! 方法逻辑：evaluate

use std::sync::Arc;

pub async fn run(
    services: Arc<assembly::Services>,
    _args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    crate::unit(services.health_evaluate().await)
}
