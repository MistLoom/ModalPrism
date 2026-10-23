//! 方法逻辑：complete

use std::sync::Arc;

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let receipt = crate::text(&args, "receiptId")?;
    crate::unit(services.paid_complete(&receipt).await)
}
