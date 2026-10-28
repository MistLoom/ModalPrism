//! 检查配置而不替换快照。

mod logic;

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec};

pub fn provider(ctx: Arc<dyn types::ReadCtx>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("config_inspect");
    spec = spec.on(
        "validate",
        "重新校验配置目录",
        serde_json::json!({"type": "object"}),
        {
            let ctx = Arc::clone(&ctx);
            move |_ctx, args| {
                let ctx = Arc::clone(&ctx);
                ability_support::box_fut(logic::validate::run(ctx, args))
            }
        },
    );
    spec = spec.on(
        "diff",
        "比较当前快照与目录",
        serde_json::json!({"type": "object"}),
        {
            let ctx = Arc::clone(&ctx);
            move |_ctx, args| {
                let ctx = Arc::clone(&ctx);
                ability_support::box_fut(logic::diff::run(ctx, args))
            }
        },
    );
    spec = spec.on(
        "explain",
        "解释模块字段来源",
        serde_json::json!({
            "type": "object",
            "properties": { "module": { "type": "string" } },
            "required": ["module"]
        }),
        {
            let ctx = Arc::clone(&ctx);
            move |_ctx, args| {
                let ctx = Arc::clone(&ctx);
                ability_support::box_fut(logic::explain::run(ctx, args))
            }
        },
    );
    spec = spec.on(
        "modules",
        "列出有效模块配置",
        serde_json::json!({"type": "object"}),
        {
            let ctx = Arc::clone(&ctx);
            move |_ctx, args| {
                let ctx = Arc::clone(&ctx);
                ability_support::box_fut(logic::modules::run(ctx, args))
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
