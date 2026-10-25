//! 方法逻辑：proxy
//!
//! 全局出站代理写在 `system.yaml` 的 `policy.egress.proxy`。空值表示移除覆盖，
//! 回到「跟随环境与系统设置」。

use std::sync::Arc;

use serde_json::json;

pub async fn run(
    ctx: Arc<dyn types::ReadCtx>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let value = args
        .get("value")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let proxy = config::proxy(ctx.config_dir(), ctx.load_options(), value).map_err(crate::fail)?;
    Ok(json!({"proxy": proxy}))
}
