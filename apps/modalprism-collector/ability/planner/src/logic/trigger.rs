//! 方法逻辑：trigger

use std::sync::Arc;

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let source = crate::text(&args, "source")?;
    crate::map(services.dispatch_round(Some(&source)).await)
}
