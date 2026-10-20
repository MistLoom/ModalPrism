//! 读取已发布的资源、facet 和变更。

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec};

pub fn provider(services: Arc<host::HostResources>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("strata_read").stateful();
    spec = spec.on(
        "kinds",
        "列出资源上的 facet 种类",
        serde_json::json!({"type": "object"}),
        {
            let services = std::sync::Arc::clone(&services);
            move |_ctx, args| {
                let services = std::sync::Arc::clone(&services);
                ability_support::box_fut(async move {
                    map(host::api::invoke(&services, "strata_read", "kinds", args).await)
                })
            }
        },
    );
    spec = spec.on(
        "has",
        "判断资源是否有某种 facet",
        serde_json::json!({"type": "object"}),
        {
            let services = std::sync::Arc::clone(&services);
            move |_ctx, args| {
                let services = std::sync::Arc::clone(&services);
                ability_support::box_fut(async move {
                    map(host::api::invoke(&services, "strata_read", "has", args).await)
                })
            }
        },
    );
    spec = spec.on(
        "get",
        "读取 facet 的版本与元数据",
        serde_json::json!({"type": "object"}),
        {
            let services = std::sync::Arc::clone(&services);
            move |_ctx, args| {
                let services = std::sync::Arc::clone(&services);
                ability_support::box_fut(async move {
                    map(host::api::invoke(&services, "strata_read", "get", args).await)
                })
            }
        },
    );
    spec = spec.on(
        "slice",
        "读取文本区间",
        serde_json::json!({"type": "object"}),
        {
            let services = std::sync::Arc::clone(&services);
            move |_ctx, args| {
                let services = std::sync::Arc::clone(&services);
                ability_support::box_fut(async move {
                    map(host::api::invoke(&services, "strata_read", "slice", args).await)
                })
            }
        },
    );
    spec = spec.on(
        "resolve",
        "按身份键解析资源",
        serde_json::json!({"type": "object"}),
        {
            let services = std::sync::Arc::clone(&services);
            move |_ctx, args| {
                let services = std::sync::Arc::clone(&services);
                ability_support::box_fut(async move {
                    map(host::api::invoke(&services, "strata_read", "resolve", args).await)
                })
            }
        },
    );
    spec = spec.on(
        "history",
        "列出 facet 版本",
        serde_json::json!({"type": "object"}),
        {
            let services = std::sync::Arc::clone(&services);
            move |_ctx, args| {
                let services = std::sync::Arc::clone(&services);
                ability_support::box_fut(async move {
                    map(host::api::invoke(&services, "strata_read", "history", args).await)
                })
            }
        },
    );
    spec = spec.on(
        "lineage",
        "读取 facet 血缘",
        serde_json::json!({"type": "object"}),
        {
            let services = std::sync::Arc::clone(&services);
            move |_ctx, args| {
                let services = std::sync::Arc::clone(&services);
                ability_support::box_fut(async move {
                    map(host::api::invoke(&services, "strata_read", "lineage", args).await)
                })
            }
        },
    );
    spec = spec.on(
        "related",
        "读取资源关系",
        serde_json::json!({"type": "object"}),
        {
            let services = std::sync::Arc::clone(&services);
            move |_ctx, args| {
                let services = std::sync::Arc::clone(&services);
                ability_support::box_fut(async move {
                    map(host::api::invoke(&services, "strata_read", "related", args).await)
                })
            }
        },
    );
    spec = spec.on(
        "query",
        "按查询对象读取",
        serde_json::json!({"type": "object"}),
        {
            let services = std::sync::Arc::clone(&services);
            move |_ctx, args| {
                let services = std::sync::Arc::clone(&services);
                ability_support::box_fut(async move {
                    map(host::api::invoke(&services, "strata_read", "query", args).await)
                })
            }
        },
    );
    spec = spec.on(
        "changes",
        "读取已可见变更",
        serde_json::json!({"type": "object"}),
        {
            let services = std::sync::Arc::clone(&services);
            move |_ctx, args| {
                let services = std::sync::Arc::clone(&services);
                ability_support::box_fut(async move {
                    map(host::api::invoke(&services, "strata_read", "changes", args).await)
                })
            }
        },
    );
    spec.into_provider()
}

fn map(result: Result<serde_json::Value, host::Error>) -> ability_support::Result<serde_json::Value> {
    host::api::map_ability(result)
}
