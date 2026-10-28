//! 执行已租约的任务。

mod logic;

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec};

pub fn provider(services: Arc<assembly::Services>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("runner").stateful();
    spec = spec.on(
        "run",
        "按任务号立即执行一个任务",
        serde_json::json!({
            "type": "object",
            "properties": { "taskId": { "type": "string" } },
            "required": ["taskId"]
        }),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::run::run(services, args))
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

fn map<T: serde::Serialize>(
    result: Result<T, assembly::Error>,
) -> ability_support::Result<serde_json::Value> {
    let value = result.map_err(failure)?;
    serde_json::to_value(value).map_err(|error| ability_support::Error::failed(error.to_string()))
}

fn parse<T: serde::de::DeserializeOwned>(args: serde_json::Value) -> ability_support::Result<T> {
    serde_json::from_value(args)
        .map_err(|error| ability_support::Error::invalid_args(error.to_string()))
}
