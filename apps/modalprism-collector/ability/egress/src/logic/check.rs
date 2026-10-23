//! 方法逻辑：check

use std::sync::Arc;

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let url = crate::text(&args, "url")?;
    crate::map(services.egress_check(&url).await)
}
