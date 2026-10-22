//! 方法逻辑：project

use std::sync::Arc;

use types::ReadRoot;

pub async fn run(
    root: Arc<ReadRoot>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    crate::map(root.call("type_hotlist.project", args).await)
}
