//! 工具动词 `strategy.ls`：列出复用策略。

pub async fn run(args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    crate::forward("strategy.ls", args).await
}
