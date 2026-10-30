//! 工具面能力：方法与参数由 `surface::TOOLS` 派生，调用转到本机回环领域 API 的
//! `POST /tools/{verb}`。
//!
//! 为什么走回环而不是进程内直调：领域 API 才是参数校验、项目隔离与审计的唯一入口，
//! 能力层只做协议投影（CLI / MCP / HTTP 三面同一套动词），不复制业务逻辑。

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec};
use serde_json::{Value, json};

mod logic;

/// 回环地址：与组合根的 `--http` 保持一致，可用 `MODALPRISM_HTTP` 覆盖。
fn base() -> String {
    std::env::var("MODALPRISM_HTTP").unwrap_or_else(|_| "http://127.0.0.1:17880".into())
}

pub fn provider() -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("tool");
    for tool in surface::TOOLS {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        for param in tool.params {
            properties.insert(
                param.name.to_string(),
                json!({
                    "type": param.kind,
                    "description": param.summary,
                }),
            );
            if param.required {
                required.push(json!(param.name));
            }
        }
        let mut input = json!({ "type": "object", "properties": properties });
        if !required.is_empty() {
            input["required"] = json!(required);
        }
        let verb = tool.id;
        spec = spec.on(verb, tool.summary, input, move |_ctx, args| {
            let verb = verb;
            ability_support::box_fut(async move {
                match verb {
                    "catalog" => logic::catalog::run(args).await,
                    "once" => logic::once::run(args).await,
                    "probe" => logic::probe::run(args).await,
                    "save" => logic::save::run(args).await,
                    "fetch" => logic::fetch::run(args).await,
                    "search" => logic::search::run(args).await,
                    "parse" => logic::parse::run(args).await,
                    "read" => logic::read::run(args).await,
                    other => Err(ability_support::Error::not_found(other)),
                }
            })
        });
    }
    spec.into_provider()
}

/// 转发到领域 API。项目上下文经请求头透传：环境变量缺省时由领域 API 回落默认项目。
pub(crate) async fn forward(verb: &str, args: Value) -> ability_support::Result<Value> {
    let client = reqwest::Client::new();
    let mut request = client.post(format!("{}/api/v1/tools/{verb}", base()));
    if let Ok(project) = std::env::var("MODALPRISM_PROJECT") {
        request = request.header("x-project-id", project);
    }
    let response = request
        .json(&args)
        .send()
        .await
        .map_err(|error| ability_support::Error::unavailable(format!("领域 API 不可达: {error}")))?;
    let status = response.status();
    let payload: Value = response
        .json()
        .await
        .map_err(|error| ability_support::Error::failed(format!("响应解析失败: {error}")))?;
    if !status.is_success() {
        let message = payload
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("工具调用失败")
            .to_string();
        if status.as_u16() == 400 {
            return Err(ability_support::Error::invalid_args(message));
        }
        return Err(ability_support::Error::failed(message));
    }
    Ok(payload)
}
