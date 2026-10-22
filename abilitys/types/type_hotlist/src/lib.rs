//! 热榜投影的 Ability。`on_commit` 返回变更集，不调用 apply。

mod logic;

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec};
use types::ReadRoot;

pub fn provider(root: Arc<ReadRoot>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("type_hotlist");
    spec = spec.on(
        "project",
        "投影一期热榜",
        serde_json::json!({"type": "object"}),
        {
            let root = Arc::clone(&root);
            move |_ctx, args| {
                let root = Arc::clone(&root);
                ability_support::box_fut(logic::project::run(root, args))
            }
        },
    );
    spec = spec.on(
        "on_commit",
        "返回名次差与保留份数的变更集",
        serde_json::json!({"type": "object"}),
        {
            let root = Arc::clone(&root);
            move |_ctx, args| {
                let root = Arc::clone(&root);
                ability_support::box_fut(logic::on_commit::run(root, args))
            }
        },
    );
    spec.into_provider()
}

fn map(
    result: Result<serde_json::Value, types::Error>,
) -> ability_support::Result<serde_json::Value> {
    result.map_err(|error| {
        if types::invalid_input(&error.message) {
            ability_support::Error::invalid_args(error.message)
        } else {
            ability_support::Error::failed(error.to_string())
        }
    })
}
