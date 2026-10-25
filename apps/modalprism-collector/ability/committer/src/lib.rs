//! 发布快照并恢复存储。

mod logic;

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec, ProtocolKind};

pub fn provider(services: Arc<assembly::Services>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("committer")
        .stateful()
        .protocols(&[ProtocolKind::Fun, ProtocolKind::Http]);
    spec = spec.on(
        "handle",
        "轮询一个长任务句柄的状态与结果",
        serde_json::json!({
            "type": "object",
            "properties": { "handle": { "type": "string" } },
            "required": ["handle"]
        }),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::handle::run(services, args))
            }
        },
    );
    spec = spec.on(
        "flush",
        "发布已接受的变更",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::flush::run(services, args))
            }
        },
    );
    spec = spec.on(
        "recover",
        "恢复存储",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::recover::run(services, args))
            }
        },
    );
    spec = spec.on(
        "status",
        "返回控制库、Blob 和暂存区路径",
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
        "sweep",
        "按保留策略清理终态任务与过期存储行；带筛选时只清理命中的终态任务",
        serde_json::json!({
            "type": "object",
            "properties": {
                "dryRun": { "type": "boolean" },
                "status": { "type": "string", "enum": ["queued", "leased", "done", "cancelled"] },
                "source": { "type": "string" },
                "queue": { "type": "string" },
                "reason": { "type": "string" },
                "query": { "type": "string" },
                "before": { "type": "string" }
            }
        }),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::sweep::run(services, args))
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

fn parse<T: serde::de::DeserializeOwned>(args: serde_json::Value) -> ability_support::Result<T> {
    serde_json::from_value(args)
        .map_err(|error| ability_support::Error::invalid_args(error.to_string()))
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

fn invalid<T>() -> ability_support::Result<T> {
    Err(ability_support::Error::invalid_args("参数无效"))
}
