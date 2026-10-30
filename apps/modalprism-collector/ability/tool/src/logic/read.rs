//! 工具动词 `read`：读取已入库内容的详情或单面数据。

pub async fn run(args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    crate::forward("read", args).await
}
