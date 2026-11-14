//! 工具动词 `ls`：发现信源与可挂载组件。

pub async fn run(args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    crate::forward("ls", args).await
}
