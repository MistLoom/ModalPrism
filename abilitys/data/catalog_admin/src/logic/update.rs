//! 方法逻辑：update

use std::sync::Arc;

use serde_json::json;

pub async fn run(
    ctx: Arc<dyn types::ReadCtx>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let draft = config::Draft::parse(&args).map_err(crate::fail)?;
    let saved =
        config::update(ctx.config_dir(), ctx.load_options(), &draft).map_err(crate::fail)?;
    Ok(json!({"id": saved.id, "file": saved.file}))
}
