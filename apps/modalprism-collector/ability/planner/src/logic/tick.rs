//! 方法逻辑：tick

use std::sync::Arc;

pub async fn run(
    services: Arc<assembly::Services>,
    _args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    crate::map(services.planner_tick().await)
}
