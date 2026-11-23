//! 工具动词 `strategy.new`：新建策略声明。

pub async fn run(args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    crate::forward("strategy.new", args).await
}
