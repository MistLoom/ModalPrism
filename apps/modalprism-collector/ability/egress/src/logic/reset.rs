//! 方法逻辑：reset

use std::sync::Arc;

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let host = crate::text(&args, "host")?;
    let existed = services.egress_reset(&host).await;
    crate::map(existed.map(|existed| serde_json::json!({ "host": host, "existed": existed })))
}
