//! 文件访问。

mod logic;

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec};

pub fn provider(services: Arc<assembly::Services>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("accessor_file");
    spec = spec.on(
        "validate",
        "校验访问规格",
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
        "access",
        "执行一次访问",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::access::run(services, args))
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
