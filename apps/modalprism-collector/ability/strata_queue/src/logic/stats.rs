//! 方法逻辑：stats

use std::sync::Arc;

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    if !args.is_null() && args.as_object().is_some_and(|map| !map.is_empty()) {
        return crate::invalid();
    }
    crate::map(services.queue_stats().await)
}
