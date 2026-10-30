//! 工具动词 `once`：取数 → 识别 → 适配 → 写入或返回。

pub async fn run(args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    crate::forward("once", args).await
}
