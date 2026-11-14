//! 工具动词 `plan.run`：立即触发一次信源采集。

pub async fn run(args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    crate::forward("plan.run", args).await
}
