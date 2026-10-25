//! 方法逻辑：toggle

use std::sync::Arc;

pub async fn run(
    engine: Arc<workflow::Engine>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let slug = crate::text(&args, "slug")?;
    let enabled = crate::flag(&args, "enabled")?;
    engine
        .store()
        .toggle_workflow(&slug, enabled)
        .map_err(crate::fail)
}
