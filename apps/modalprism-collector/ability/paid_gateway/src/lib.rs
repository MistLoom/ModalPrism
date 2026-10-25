//! 付费回执。

mod logic;

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec, ProtocolKind};

pub fn provider(services: Arc<assembly::Services>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("paid_gateway")
        .stateful()
        .protocols(&[ProtocolKind::Fun]);
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

fn unit(result: Result<(), assembly::Error>) -> ability_support::Result<serde_json::Value> {
    result.map_err(failure)?;
    Ok(serde_json::json!({}))
}

fn text(args: &serde_json::Value, key: &str) -> ability_support::Result<String> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| ability_support::Error::invalid_args(format!("缺少字符串参数 {key}")))
}
