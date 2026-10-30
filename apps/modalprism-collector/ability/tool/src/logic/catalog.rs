//! 工具动词 `catalog`：列出全部动词与参数。

pub async fn run(args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    crate::forward("catalog", args).await
}
