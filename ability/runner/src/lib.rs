//! 执行已租约的任务。

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec};

pub fn provider(services: Arc<host::HostResources>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("runner").stateful();
    spec = spec.on(
        "run_task",
        "执行一个任务",
        serde_json::json!({"type": "object"}),
        {
            let services = std::sync::Arc::clone(&services);
            move |_ctx, args| {
                let services = std::sync::Arc::clone(&services);
                ability_support::box_fut(async move {
                    map(host::api::invoke(&services, "runner", "run_task", args).await)
                })
            }
        },
    );
    spec.into_provider()
}

fn map(result: Result<serde_json::Value, host::Error>) -> ability_support::Result<serde_json::Value> {
    host::api::map_ability(result)
}
