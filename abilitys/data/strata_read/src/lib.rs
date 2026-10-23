//! 读取已发布的资源、facet 和变更。

mod logic;

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec};

pub fn provider(ctx: Arc<dyn types::ReadCtx>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("strata_read").stateful();
    spec = spec.on(
        "kinds",
        "列出资源上的 facet 种类",
        serde_json::json!({"type": "object"}),
        {
            let ctx = Arc::clone(&ctx);
            move |_ctx, args| {
                let ctx = Arc::clone(&ctx);
                ability_support::box_fut(logic::kinds::run(ctx, args))
            }
        },
    );
    spec = spec.on(
        "has",
        "判断资源是否有某种 facet",
        serde_json::json!({"type": "object"}),
        {
            let ctx = Arc::clone(&ctx);
            move |_ctx, args| {
                let ctx = Arc::clone(&ctx);
                ability_support::box_fut(logic::has::run(ctx, args))
            }
        },
    );
    spec = spec.on(
        "get",
        "读取 facet 的版本与元数据",
        serde_json::json!({"type": "object"}),
        {
            let ctx = Arc::clone(&ctx);
            move |_ctx, args| {
                let ctx = Arc::clone(&ctx);
                ability_support::box_fut(logic::get::run(ctx, args))
            }
        },
    );
    spec = spec.on(
        "slice",
        "读取文本区间",
        serde_json::json!({"type": "object"}),
        {
            let ctx = Arc::clone(&ctx);
            move |_ctx, args| {
                let ctx = Arc::clone(&ctx);
                ability_support::box_fut(logic::slice::run(ctx, args))
            }
        },
    );
    spec = spec.on(
        "resolve",
        "按身份键解析资源",
        serde_json::json!({"type": "object"}),
        {
            let ctx = Arc::clone(&ctx);
            move |_ctx, args| {
                let ctx = Arc::clone(&ctx);
                ability_support::box_fut(logic::resolve::run(ctx, args))
            }
        },
    );
    spec = spec.on(
        "history",
        "列出 facet 版本",
        serde_json::json!({"type": "object"}),
        {
            let ctx = Arc::clone(&ctx);
            move |_ctx, args| {
                let ctx = Arc::clone(&ctx);
                ability_support::box_fut(logic::history::run(ctx, args))
            }
        },
    );
    spec = spec.on(
        "lineage",
        "读取 facet 血缘",
        serde_json::json!({"type": "object"}),
        {
            let ctx = Arc::clone(&ctx);
            move |_ctx, args| {
                let ctx = Arc::clone(&ctx);
                ability_support::box_fut(logic::lineage::run(ctx, args))
            }
        },
    );
    spec = spec.on(
        "related",
        "读取资源关系",
        serde_json::json!({"type": "object"}),
        {
            let ctx = Arc::clone(&ctx);
            move |_ctx, args| {
                let ctx = Arc::clone(&ctx);
                ability_support::box_fut(logic::related::run(ctx, args))
            }
        },
    );
    spec = spec.on(
        "query",
        "按查询对象读取",
        serde_json::json!({"type": "object"}),
        {
            let ctx = Arc::clone(&ctx);
            move |_ctx, args| {
                let ctx = Arc::clone(&ctx);
                ability_support::box_fut(logic::query::run(ctx, args))
            }
        },
    );
    spec = spec.on(
        "changes",
        "读取已可见变更",
        serde_json::json!({"type": "object"}),
        {
            let ctx = Arc::clone(&ctx);
            move |_ctx, args| {
                let ctx = Arc::clone(&ctx);
                ability_support::box_fut(logic::changes::run(ctx, args))
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
