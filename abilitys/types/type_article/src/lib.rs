//! 文章投影的 Ability。调用落在 `types::ReadRoot` 上。

mod logic;

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec};
use types::ReadRoot;

pub fn provider(root: Arc<ReadRoot>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("type_article");
    spec = spec.on(
        "project",
        "投影一篇文章",
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
        "文章不产出变更集",
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
