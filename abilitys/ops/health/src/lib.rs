//! 记录信源健康。

mod logic;

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec};

pub fn provider(ctx: Arc<dyn types::ReadCtx>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("health").stateful();
    spec = spec.on(
        "status",
        "读取信源健康",
        serde_json::json!({
            "type": "object",
            "properties": { "source": { "type": "string" } },
            "required": ["source"]
        }),
        {
            let ctx = Arc::clone(&ctx);
            move |_ctx, args| {
                let ctx = Arc::clone(&ctx);
                ability_support::box_fut(logic::status::run(ctx, args))
            }
        },
    );
    spec = spec.on(
        "report",
        "读取信源、通道与解析器的运行态",
        serde_json::json!({"type": "object"}),
        {
            let ctx = Arc::clone(&ctx);
            move |_ctx, args| {
                let ctx = Arc::clone(&ctx);
                ability_support::box_fut(logic::report::run(ctx, args))
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
