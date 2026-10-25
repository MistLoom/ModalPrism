//! 方法逻辑：toggle

use std::sync::Arc;

use serde_json::json;

pub async fn run(
    ctx: Arc<dyn types::ReadCtx>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let kind = crate::text(&args, "kind")?;
    let slug = crate::text(&args, "slug")?;
    let enabled = crate::flag(&args, "enabled")?;
    let saved = config::toggle(ctx.config_dir(), ctx.load_options(), &kind, &slug, enabled)
        .map_err(crate::fail)?;
    Ok(json!({"id": saved.id, "file": saved.file, "enabled": enabled}))
}
