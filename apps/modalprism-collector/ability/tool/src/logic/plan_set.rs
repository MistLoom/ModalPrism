//! 工具动词 `plan.set`：修改采集计划。

pub async fn run(args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    crate::forward("plan.set", args).await
}
