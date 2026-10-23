//! 方法逻辑：status

use std::sync::Arc;

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let receipt = crate::text(&args, "receiptId")?;
    let status = services.paid_status(&receipt).await;
    crate::map(status.map(|status| serde_json::json!({ "status": status })))
}
