//! 方法逻辑：remove

use std::sync::Arc;

pub async fn run(
    engine: Arc<workflow::Engine>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let slug = crate::text(&args, "slug")?;
    engine.store().remove_workflow(&slug).map_err(crate::fail)
}
