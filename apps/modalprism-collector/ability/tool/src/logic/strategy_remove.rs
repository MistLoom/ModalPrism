//! 工具动词 `strategy.remove`：删除策略声明。

pub async fn run(args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    crate::forward("strategy.remove", args).await
}
