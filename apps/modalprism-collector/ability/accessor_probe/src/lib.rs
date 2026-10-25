//! 按实现名校验 accessor 配置。
//!
//! 配置编辑期只需要「这份 spec 能不能被该实现接受」，不值得为每个实现开一个能力。

mod logic;

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec, ProtocolKind};

pub fn provider(services: Arc<assembly::Services>) -> Arc<dyn AbilityProvider> {
    let mut spec =
        AbilitySpec::must("accessor_probe").protocols(&[ProtocolKind::Fun, ProtocolKind::Http]);
    spec = spec.on(
        "validate",
        "用 implementation 与 spec 校验一份 accessor 配置",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::validate::run(services, args))
            }
        },
    );
    spec.into_provider()
}

fn unit(result: Result<(), assembly::Error>) -> ability_support::Result<serde_json::Value> {
    match result {
        Ok(()) => Ok(serde_json::json!({})),
        Err(error) => Err(failure(error)),
    }
}

fn parse<T: serde::de::DeserializeOwned>(args: serde_json::Value) -> ability_support::Result<T> {
    serde_json::from_value(args).map_err(|_| ability_support::Error::invalid_args("spec 无效"))
}

fn failure(error: assembly::Error) -> ability_support::Error {
    if assembly::invalid_input(&error.message) {
        ability_support::Error::invalid_args(error.message)
    } else {
        ability_support::Error::failed(error.to_string())
    }
}
