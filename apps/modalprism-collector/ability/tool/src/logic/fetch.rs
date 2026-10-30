//! 工具动词 `fetch`：按地址取回正文。

pub async fn run(args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    crate::forward("fetch", args).await
}
