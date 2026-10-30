//! 工具动词 `search`：用已配置的搜索引擎检索。

pub async fn run(args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    crate::forward("search", args).await
}
