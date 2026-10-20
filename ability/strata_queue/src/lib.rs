//! 控制库任务队列。

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec};

pub fn provider(services: Arc<host::HostResources>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("strata_queue").stateful();
    spec = spec.on(
        "enqueue",
        "写入任务",
        serde_json::json!({"type": "object"}),
        {
            let services = std::sync::Arc::clone(&services);
            move |_ctx, args| {
                let services = std::sync::Arc::clone(&services);
                ability_support::box_fut(async move {
                    map(host::api::invoke(&services, "strata_queue", "enqueue", args).await)
                })
            }
        },
    );
    spec = spec.on(
        "lease",
        "租约一批任务",
        serde_json::json!({"type": "object"}),
        {
            let services = std::sync::Arc::clone(&services);
            move |_ctx, args| {
                let services = std::sync::Arc::clone(&services);
                ability_support::box_fut(async move {
                    map(host::api::invoke(&services, "strata_queue", "lease", args).await)
                })
            }
        },
    );
    spec = spec.on(
        "extend",
        "延长租约",
        serde_json::json!({"type": "object"}),
        {
            let services = std::sync::Arc::clone(&services);
            move |_ctx, args| {
                let services = std::sync::Arc::clone(&services);
                ability_support::box_fut(async move {
                    map(host::api::invoke(&services, "strata_queue", "extend", args).await)
                })
            }
        },
    );
    spec = spec.on(
        "nack",
        "退回任务",
        serde_json::json!({"type": "object"}),
        {
            let services = std::sync::Arc::clone(&services);
            move |_ctx, args| {
                let services = std::sync::Arc::clone(&services);
                ability_support::box_fut(async move {
                    map(host::api::invoke(&services, "strata_queue", "nack", args).await)
                })
            }
        },
    );
    spec.into_provider()
}

fn map(result: Result<serde_json::Value, host::Error>) -> ability_support::Result<serde_json::Value> {
    host::api::map_ability(result)
}
