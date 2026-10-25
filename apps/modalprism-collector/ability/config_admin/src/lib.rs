//! 加载配置快照。

mod logic;

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec, ProtocolKind};

pub fn provider(services: Arc<assembly::Services>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("config_admin")
        .stateful()
        .protocols(&[ProtocolKind::Fun, ProtocolKind::Http]);
    spec = spec.on(
        "load",
        "重新读取配置目录",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::load::run(services, args))
            }
        },
    );
    spec = spec.on(
        "generation",
        "返回当前快照代数",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::generation::run(services, args))
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
