//! 方法逻辑：next

use std::sync::Arc;

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let source = crate::text(&args, "source")?;
    let status = services.planner_next(&source).await;
    crate::map(status.map(|status| super::status::project(&status)))
}
