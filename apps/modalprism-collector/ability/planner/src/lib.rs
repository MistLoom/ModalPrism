//! 扫描到期信源并租约任务。

mod logic;

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec, ProtocolKind};

pub fn provider(services: Arc<assembly::Services>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("planner")
        .stateful()
        .protocols(&[ProtocolKind::Fun, ProtocolKind::Http]);
    spec = spec.on(
        "tick",
        "派出到期任务",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::tick::run(services, args))
            }
        },
    );
    spec = spec.on(
        "trigger",
        "把信源标成到期并立即采集一轮",
        serde_json::json!({
            "type": "object",
            "properties": { "source": { "type": "string" } },
            "required": ["source"]
        }),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::trigger::run(services, args))
            }
        },
    );
    spec = spec.on(
        "round",
        "执行一轮派发",
        serde_json::json!({
            "type": "object",
            "properties": { "source": { "type": "string" } }
        }),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::round::run(services, args))
            }
        },
    );
    spec = spec.on(
        "manual",
        "为信源新建一条采集任务",
        serde_json::json!({
            "type": "object",
            "properties": {
                "source": { "type": "string" },
                "queue": { "type": "string" },
                "runNow": { "type": "boolean" },
                "locator": { "type": "string" }
            },
            "required": ["source"]
        }),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::manual::run(services, args))
            }
        },
    );
    spec = spec.on(
        "queues",
        "每队列的排队、租约与暂停状态",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::queues::run(services, args))
            }
        },
    );
    spec = spec.on(
        "pause",
        "暂停或恢复一个队列",
        serde_json::json!({
            "type": "object",
            "properties": {
                "queue": { "type": "string" },
                "paused": { "type": "boolean" }
            },
            "required": ["queue", "paused"]
        }),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::pause::run(services, args))
            }
        },
    );
    spec = spec.on(
        "next",
        "读取信源的下一次时间",
        serde_json::json!({
            "type": "object",
            "properties": { "source": { "type": "string" } },
            "required": ["source"]
        }),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::next::run(services, args))
            }
        },
    );
    spec = spec.on(
        "status",
        "列出所有信源的调度与健康状态",
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

fn text(args: &serde_json::Value, key: &str) -> ability_support::Result<String> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| ability_support::Error::invalid_args(format!("缺少字符串参数 {key}")))
}

fn parse<T: serde::de::DeserializeOwned>(args: serde_json::Value) -> ability_support::Result<T> {
    serde_json::from_value(args)
        .map_err(|error| ability_support::Error::invalid_args(error.to_string()))
}
