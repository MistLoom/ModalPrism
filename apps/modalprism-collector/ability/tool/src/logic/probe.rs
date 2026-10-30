//! 工具动词 `probe`：只识别不落库。

pub async fn run(args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    crate::forward("probe", args).await
}
