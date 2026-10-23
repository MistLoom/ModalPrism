//! 方法逻辑：flush

use std::sync::Arc;

pub async fn run(
    services: Arc<assembly::Services>,
    _args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    crate::map(
        services
            .committer_flush()
            .await
            .map(|snapshot| serde_json::json!({ "snapshot": snapshot })),
    )
}
