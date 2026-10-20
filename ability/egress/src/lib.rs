//! 出站检查与正向代理状态。

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec};

pub fn provider(services: Arc<host::HostResources>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("egress").stateful();
    spec = spec.on(
        "check",
        "检查地址是否被私网规则挡住",
        serde_json::json!({"type": "object"}),
        {
            let services = std::sync::Arc::clone(&services);
            move |_ctx, args| {
                let services = std::sync::Arc::clone(&services);
                ability_support::box_fut(
                    async move { map(host::api::invoke(&services, "egress", "check", args).await) },
                )
            }
        },
    );
    spec = spec.on(
        "status",
        "返回正向代理地址",
        serde_json::json!({"type": "object"}),
        {
            let services = std::sync::Arc::clone(&services);
            move |_ctx, args| {
                let services = std::sync::Arc::clone(&services);
                ability_support::box_fut(
                    async move { map(host::api::invoke(&services, "egress", "status", args).await) },
                )
            }
        },
    );
    spec.into_provider()
}

fn map(result: Result<serde_json::Value, host::Error>) -> ability_support::Result<serde_json::Value> {
    host::api::map_ability(result)
}
