//! 方法逻辑：generation

use std::sync::Arc;

pub async fn run(
    services: Arc<assembly::Services>,
    _args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    crate::map(Ok(
        serde_json::json!({ "generation": services.config_generation() }),
    ))
}
