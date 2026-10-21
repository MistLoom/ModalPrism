// 自动生成，勿手改
use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec};

use super::bridge::instance;
use super::manifest::{ABILITY_ID, DECL_FRAMEWORK_VERSION};

macro_rules! handler_impl {
    ($method:ident) => {
        move |ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value| {
            let handlers = instance();
            ability_support::box_fut(async move {
                handlers.$method(ctx, args).await
            })
        }
    };
}

pub fn provider() -> Arc<dyn AbilityProvider> {
    assert_eq!(
        DECL_FRAMEWORK_VERSION,
        ability_support::FRAMEWORK_VERSION,
        "ability.yml framework_version 与当前框架不匹配"
    );
    AbilitySpec::try_new(ABILITY_ID).expect("id")
        .stateful()
        .on(
            "kinds",
            "列出资源上的 facet 种类",
            serde_json::json!({"properties":{"includeVariants":{"type":"boolean"},"rid":{"type":"string"}},"required":["rid"],"type":"object"}),
            handler_impl!(kinds),
        )
        .on(
            "has",
            "判断资源是否有某种 facet",
            serde_json::json!({"properties":{"kind":{"type":"string"},"rid":{"type":"string"},"variant":{"type":"string"}},"required":["rid","kind"],"type":"object"}),
            handler_impl!(has),
        )
        .on(
            "get",
            "读取 facet 的版本与元数据",
            serde_json::json!({"properties":{"facet":{"type":"string"},"lazy":{"type":"boolean"}},"required":["facet"],"type":"object"}),
            handler_impl!(get),
        )
        .on(
            "slice",
            "读取文本区间",
            serde_json::json!({"properties":{"span":{"type":"string"}},"required":["span"],"type":"object"}),
            handler_impl!(slice),
        )
        .on(
            "resolve",
            "按身份键解析资源",
            serde_json::json!({"properties":{"identity":{"type":"string"}},"required":["identity"],"type":"object"}),
            handler_impl!(resolve),
        )
        .on(
            "history",
            "列出 facet 版本",
            serde_json::json!({"properties":{"facet":{"type":"string"}},"required":["facet"],"type":"object"}),
            handler_impl!(history),
        )
        .on(
            "lineage",
            "读取 facet 血缘",
            serde_json::json!({"properties":{"depth":{"type":"integer"},"direction":{"type":"string"},"facet":{"type":"string"}},"required":["facet"],"type":"object"}),
            handler_impl!(lineage),
        )
        .on(
            "related",
            "读取资源关系",
            serde_json::json!({"properties":{"direction":{"type":"string"},"rel":{"type":"string"},"rid":{"type":"string"}},"required":["rid"],"type":"object"}),
            handler_impl!(related),
        )
        .on(
            "query",
            "按查询对象读取",
            serde_json::json!({"properties":{"query":{"type":"object"}},"required":["query"],"type":"object"}),
            handler_impl!(query),
        )
        .on(
            "changes",
            "读取已可见变更",
            serde_json::json!({"properties":{"after":{"type":"integer"},"limit":{"type":"integer"}},"required":["after"],"type":"object"}),
            handler_impl!(changes),
        )
        .into_provider()
}
