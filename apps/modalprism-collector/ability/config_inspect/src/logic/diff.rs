//! 方法逻辑：diff

use std::sync::Arc;

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    crate::map(services.call("config_inspect.diff", args).await)
}
