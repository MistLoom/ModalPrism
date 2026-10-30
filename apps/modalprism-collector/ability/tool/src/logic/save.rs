//! 工具动词 `save`：把探测结论固化为信源声明。

pub async fn run(args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    crate::forward("save", args).await
}
