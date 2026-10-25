//! 方法逻辑：handle —— 句柄轮询的正常路径由运维门面短路；
//! 走到这里说明门面未启用切面或句柄非法，一律报参数无效。

use std::sync::Arc;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Args {
    handle: String,
}

pub async fn run(
    _services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let args: Args = crate::parse(args)?;
    if args.handle.trim().is_empty() {
        return Err(ability_support::Error::invalid_args("参数无效"));
    }
    Err(ability_support::Error::failed(
        "句柄不存在或已过期，长任务结果保留 30 分钟",
    ))
}
