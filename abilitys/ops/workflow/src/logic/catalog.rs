//! 方法逻辑：catalog

use std::sync::Arc;

use serde_json::json;

pub async fn run(
    engine: Arc<workflow::Engine>,
    _args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let store = engine.store();
    Ok(json!({
        "workflows": store.workflows().map_err(crate::fail)?,
        "models": store.models().map_err(crate::fail)?,
        "templates": workflow::Store::templates(),
    }))
}
