//! 付费回执。

mod logic;

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec};

pub fn provider(services: Arc<assembly::Services>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("paid_gateway").stateful();
    spec = spec.on(
        "reserve",
        "占位方法，不扣费",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::reserve::run(services, args))
            }
        },
    );
    spec = spec.on(
        "complete",
        "确认回执",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::complete::run(services, args))
            }
        },
    );
    spec = spec.on(
        "status",
        "读取回执状态",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::status::run(services, args))
            }
        },
    );
    spec.into_provider()
}

fn map(
    result: Result<serde_json::Value, assembly::Error>,
) -> ability_support::Result<serde_json::Value> {
    result.map_err(|error| {
        if assembly::invalid_input(&error.message) {
            ability_support::Error::invalid_args(error.message)
        } else {
            ability_support::Error::failed(error.to_string())
        }
    })
}
