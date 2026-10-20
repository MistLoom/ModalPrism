//! 扫描到期信源并租约任务。

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec};

pub fn provider(services: Arc<host::HostResources>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("planner").stateful();
    spec = spec.on(
        "tick",
        "派出到期任务",
        serde_json::json!({"type": "object"}),
        {
            let services = std::sync::Arc::clone(&services);
            move |_ctx, args| {
                let services = std::sync::Arc::clone(&services);
                ability_support::box_fut(
                    async move { map(host::api::invoke(&services, "planner", "tick", args).await) },
                )
            }
        },
    );
    spec = spec.on(
        "trigger",
        "把信源标成到期并派出",
        serde_json::json!({"type": "object"}),
        {
            let services = std::sync::Arc::clone(&services);
            move |_ctx, args| {
                let services = std::sync::Arc::clone(&services);
                ability_support::box_fut(async move {
                    map(host::api::invoke(&services, "planner", "trigger", args).await)
                })
            }
        },
    );
    spec = spec.on(
        "next",
        "读取信源的下一次时间",
        serde_json::json!({"type": "object"}),
        {
            let services = std::sync::Arc::clone(&services);
            move |_ctx, args| {
                let services = std::sync::Arc::clone(&services);
                ability_support::box_fut(
                    async move { map(host::api::invoke(&services, "planner", "next", args).await) },
                )
            }
        },
    );
    spec.into_provider()
}

fn map(result: Result<serde_json::Value, host::Error>) -> ability_support::Result<serde_json::Value> {
    host::api::map_ability(result)
}
