//! 控制库任务队列。

mod logic;

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec};

pub fn provider(services: Arc<assembly::Services>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("strata_queue").stateful();
    spec = spec.on(
        "enqueue",
        "写入任务",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::enqueue::run(services, args))
            }
        },
    );
    spec = spec.on(
        "lease",
        "租约一批任务",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::lease::run(services, args))
            }
        },
    );
    spec = spec.on(
        "extend",
        "延长租约",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::extend::run(services, args))
            }
        },
    );
    spec = spec.on(
        "nack",
        "退回任务",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::nack::run(services, args))
            }
        },
    );
    spec = spec.on(
        "list",
        "按状态与队列查看任务",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::list::run(services, args))
            }
        },
    );
    spec = spec.on(
        "retry",
        "重试一个排队中的任务",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::retry::run(services, args))
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

fn parse<T: serde::de::DeserializeOwned>(args: serde_json::Value) -> ability_support::Result<T> {
    serde_json::from_value(args)
        .map_err(|error| ability_support::Error::invalid_args(error.to_string()))
}

fn invalid<T>() -> ability_support::Result<T> {
    Err(ability_support::Error::invalid_args("参数无效"))
}
