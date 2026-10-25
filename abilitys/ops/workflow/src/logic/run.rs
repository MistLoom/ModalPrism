//! 方法逻辑：run

use std::sync::Arc;

pub async fn run(
    engine: Arc<workflow::Engine>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let slug = crate::text(&args, "slug")?;
    let rid = crate::text(&args, "rid")?;
    engine.run(&slug, &rid).await.map_err(crate::fail)
}
