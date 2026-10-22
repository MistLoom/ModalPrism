//! 检查配置而不替换快照。

mod logic;

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec};

pub fn provider(services: Arc<assembly::Services>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("config_inspect");
    spec = spec.on(
        "validate",
        "重新校验配置目录",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::validate::run(services, args))
            }
        },
    );
    spec = spec.on(
        "diff",
        "比较当前快照与目录",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::diff::run(services, args))
            }
        },
    );
    spec = spec.on(
        "explain",
        "解释模块字段来源",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::explain::run(services, args))
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
