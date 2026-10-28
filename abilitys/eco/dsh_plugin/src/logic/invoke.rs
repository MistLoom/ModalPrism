use ability_support::{AbilityCtx, Error, Result};
use serde::Deserialize;
use serde_json::Value;

use super::lifecycle::runtime_handle;

#[derive(Deserialize)]
struct InvokeArgs {
    tool: String,
    #[serde(default)]
    args: Value,
}

pub async fn invoke(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let a: InvokeArgs = serde_json::from_value(args)
        .map_err(|e| Error::invalid_args(format!("invoke 参数无效: {e}")))?;
    let rt = runtime_handle(ctx.instance())?;
    let guard = rt.lock().await;
    let bridge = guard
        .bridge_async()
        .await
        .ok_or_else(|| Error::failed("DSH 宿主未启动"))?;
    bridge.call_hook(&a.tool, a.args).await
}
