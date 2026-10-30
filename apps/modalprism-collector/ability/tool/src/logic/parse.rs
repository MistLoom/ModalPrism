//! 工具动词 `parse`：对文本或地址做结构化识别。

pub async fn run(args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    crate::forward("parse", args).await
}
