//! 记录信源健康。

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec};

pub fn provider(services: Arc<host::HostResources>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("health").stateful();
    spec = spec.on(
        "evaluate",
        "把健康写入存储",
        serde_json::json!({"type": "object"}),
        {
            let services = std::sync::Arc::clone(&services);
            move |_ctx, args| {
                let services = std::sync::Arc::clone(&services);
                ability_support::box_fut(async move {
                    map(host::api::invoke(&services, "health", "evaluate", args).await)
                })
            }
        },
    );
    spec = spec.on(
        "status",
        "读取信源健康",
        serde_json::json!({"type": "object"}),
        {
            let services = std::sync::Arc::clone(&services);
            move |_ctx, args| {
                let services = std::sync::Arc::clone(&services);
                ability_support::box_fut(
                    async move { map(host::api::invoke(&services, "health", "status", args).await) },
                )
            }
        },
    );
    spec.into_provider()
}

fn map(result: Result<serde_json::Value, host::Error>) -> ability_support::Result<serde_json::Value> {
    host::api::map_ability(result)
}
