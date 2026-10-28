//! 记录信源健康。

mod logic;

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec};

pub fn provider(services: Arc<assembly::Services>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("health_record").stateful();
    spec = spec.on(
        "evaluate",
        "把健康写入存储",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::evaluate::run(services, args))
            }
        },
    );
    spec.into_provider()
}

/// 装配错误到能力错误的唯一映射。
fn failure(error: assembly::Error) -> ability_support::Error {
    if assembly::invalid_input(&error.message) {
        ability_support::Error::invalid_args(error.message)
    } else {
        ability_support::Error::failed(error.to_string())
    }
}

fn unit(result: Result<(), assembly::Error>) -> ability_support::Result<serde_json::Value> {
    result.map_err(failure)?;
    Ok(serde_json::json!({}))
}
