//! 内容治理：按筛选统计、写入墓碑与恢复。
//!
//! 删除是墓碑不是物理删除，查询默认过滤墓碑，撤销后立刻回到默认列表。
//! 这一层只解析入参与映射错误，真实的数据面调用都在装配层。

mod logic;

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec, ProtocolKind};
use assembly::domain::Timestamp;

pub fn provider(services: Arc<assembly::Services>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("content_admin")
        .stateful()
        .protocols(&[ProtocolKind::Fun, ProtocolKind::Http]);
    spec = spec.on(
        "count",
        "统计匹配的内容条数与其中已删除的条数",
        serde_json::json!({
            "type": "object",
            "properties": {
                "source": { "type": "string" },
                "before": { "type": "string" },
                "after": { "type": "string" },
                "includeTombstoned": { "type": "boolean" }
            }
        }),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::count::run(services, args))
            }
        },
    );
    spec = spec.on(
        "purge",
        "按筛选或选中行写入墓碑",
        serde_json::json!({
            "type": "object",
            "properties": {
                "rids": { "type": "array", "items": { "type": "string" } },
                "source": { "type": "string" },
                "before": { "type": "string" },
                "after": { "type": "string" },
                "reason": { "type": "string" },
                "dryRun": { "type": "boolean" }
            }
        }),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::purge::run(services, args))
            }
        },
    );
    spec = spec.on(
        "label",
        "为用户分类写入或撤销标签",
        serde_json::json!({
            "type": "object",
            "properties": {
                "rids": { "type": "array", "items": { "type": "string" } },
                "set": { "type": "object" },
                "unset": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["rids"]
        }),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::label::run(services, args))
            }
        },
    );
    spec = spec.on(
        "restore",
        "撤销墓碑，把内容放回默认列表",
        serde_json::json!({
            "type": "object",
            "properties": {
                "rids": { "type": "array", "items": { "type": "string" } },
                "reason": { "type": "string" }
            },
            "required": ["rids"]
        }),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(logic::restore::run(services, args))
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

fn invalid<T>() -> ability_support::Result<T> {
    Err(ability_support::Error::invalid_args("参数无效"))
}

/// 筛选字段在 count 与 purge 之间共用，只在这里翻译一次。
pub(crate) fn filter(
    source: Option<String>,
    before: Option<String>,
    after: Option<String>,
) -> ability_support::Result<assembly::ContentFilter> {
    Ok(assembly::ContentFilter {
        source: non_empty(source),
        before: time(before)?,
        after: time(after)?,
    })
}

pub(crate) fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.trim().is_empty())
}

pub(crate) fn time(value: Option<String>) -> ability_support::Result<Option<Timestamp>> {
    let Some(text) = non_empty(value) else {
        return Ok(None);
    };
    Timestamp::parse(&text)
        .map(Some)
        .map_err(|error| ability_support::Error::invalid_args(error.to_string()))
}
