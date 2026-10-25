//! 方法逻辑：runs

use std::sync::Arc;

use serde_json::Value;

pub async fn run(
    engine: Arc<workflow::Engine>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(20)
        .clamp(1, 200) as u32;
    engine.runs(limit).await.map_err(crate::fail)
}
