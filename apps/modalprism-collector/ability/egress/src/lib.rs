//! 出站检查与正向代理状态。

mod logic;

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec, ProtocolKind};

pub fn provider(services: Arc<assembly::Services>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("egress")
        .stateful()
        .protocols(&[ProtocolKind::Fun, ProtocolKind::Http]);
    spec = spec.on(
        "check",
        "检查地址是否被私网规则挡住",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::check::run(services, args))
            }
        },
    );
    spec = spec.on(
        "status",
        "返回正向代理与上游代理状态",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::status::run(services, args))
            }
        },
    );
    spec = spec.on(
        "report",
        "返回 robots、熔断、令牌桶与审计视图",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::report::run(services, args))
            }
        },
    );
    spec = spec.on(
        "reset",
        "重置一台主机的熔断",
        serde_json::json!({
            "type": "object",
            "required": ["host"],
            "properties": {"host": {"type": "string"}}
        }),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::reset::run(services, args))
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

fn text(args: &serde_json::Value, key: &str) -> ability_support::Result<String> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| ability_support::Error::invalid_args(format!("缺少字符串参数 {key}")))
}
