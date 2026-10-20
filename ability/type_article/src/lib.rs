//! 文章投影的 Ability。调用落在 `types::ReadRoot` 上。

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec};
use types::ReadRoot;

pub fn provider(root: Arc<ReadRoot>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("type_article");
    spec = bind(spec, &root, "project", "投影一篇文章");
    spec = bind(spec, &root, "on_commit", "文章不产出变更集");
    spec.into_provider()
}

fn bind(
    spec: AbilitySpec,
    root: &Arc<ReadRoot>,
    method: &'static str,
    description: &'static str,
) -> AbilitySpec {
    let root = Arc::clone(root);
    spec.on(
        method,
        description,
        serde_json::json!({"type": "object"}),
        move |_ctx, args| {
            let root = Arc::clone(&root);
            ability_support::box_fut(async move {
                map(root.call(&format!("type_article.{method}"), args).await)
            })
        },
    )
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
