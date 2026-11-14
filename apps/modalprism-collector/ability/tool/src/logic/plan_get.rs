//! 工具动词 `plan.get`：读取采集计划详情。

pub async fn run(args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    crate::forward("plan.get", args).await
}
