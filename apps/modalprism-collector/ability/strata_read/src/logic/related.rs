//! 方法逻辑：related

use std::sync::Arc;

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    crate::map(services.call("strata_read.related", args).await)
}
