//! 工具动词 `strategy.get`：读取策略详情。

pub async fn run(args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    crate::forward("strategy.get", args).await
}
