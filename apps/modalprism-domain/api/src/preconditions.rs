//! 前置校验：缺模型 / 密钥 / 引擎 / 可达性时返回结构化 `precondition_failed`
//! （缺什么、去哪配），勿只给裸 409。`GET /preconditions?scope=` 供向导门闩
//! 与对外表面在动作前探测就绪度。

use axum::Router;
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::util;
use crate::{AppState, Shared};

pub fn routes() -> Router<Shared> {
    Router::new().route("/preconditions", get(preconditions))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScopeQuery {
    #[serde(default)]
    scope: Option<String>,
}

/// 单条就绪项：`met` 为 false 时附 `requirements`（缺什么、去哪配）。
async fn scope_report(state: &AppState, scope: &str) -> Result<Value, ApiError> {
    match scope {
        // LLM 路径：投递到模型、信源分析与模型自检都需要已配模型。
        "llm" => {
            let model_id = crate::settings::value(state, "ai.defaultModelId")
                .await
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_default();
            let model_docs = state.store.list_documents("model").await?;
            let met = !model_id.is_empty() || !model_docs.is_empty();
            Ok(json!({
                "scope": "llm",
                "met": met,
                "requirements": if met { vec![] } else { vec![json!({
                    "type": "model",
                    "setting": "ai.defaultModelId",
                    "message": "尚未配置模型：投递到模型与模型自检会失败",
                    "where": "设置 → 模型",
                })]},
            }))
        }
        // 搜索路径：默认引擎就绪即可；密钥类引擎另查引用名。
        "search" => {
            let engine = crate::settings::value(state, "search.defaultEngine")
                .await
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "ddg".to_string());
            let catalog = crate::search::engine_catalog();
            let entry = catalog.iter().find(|entry| entry["id"] == json!(engine));
            let mut requirements = Vec::new();
            if entry.is_none() {
                requirements.push(json!({
                    "type": "engine",
                    "setting": "search.defaultEngine",
                    "message": format!("未知引擎: {engine}"),
                    "where": "设置 → 搜索",
                }));
            } else if let Some(key_field) = entry.and_then(|entry| entry["requiresKeyRef"].as_str())
            {
                let key_ref = crate::settings::value(state, key_field)
                    .await
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_string))
                    .unwrap_or_default();
                if key_ref.trim().is_empty() {
                    requirements.push(json!({
                        "type": "engine_key",
                        "engine": engine,
                        "setting": key_field,
                        "message": format!("引擎 {engine} 需要密钥引用名"),
                        "where": "设置 → 搜索",
                    }));
                }
            }
            Ok(json!({
                "scope": "search",
                "engine": engine,
                "met": requirements.is_empty(),
                "requirements": requirements,
            }))
        }
        // 采集路径：至少一个信源可用。
        "collection" => {
            let sources = crate::catalog::source_rows(state).await?;
            let enabled = sources
                .iter()
                .filter(|source| source["enabled"].as_bool().unwrap_or(false))
                .count();
            Ok(json!({
                "scope": "collection",
                "met": enabled > 0,
                "requirements": if enabled == 0 { vec![json!({
                    "type": "source",
                    "message": "尚无启用的信源",
                    "where": "信源 → 新建",
                })] } else { vec![] },
            }))
        }
        _ => Err(ApiError::validation(
            "scope",
            "invalid",
            "已知 scope: llm, search, collection",
        )),
    }
}

async fn preconditions(
    State(state): State<Shared>,
    Query(query): Query<ScopeQuery>,
) -> Result<Response, ApiError> {
    let scopes: Vec<String> = match query.scope.as_deref() {
        Some(raw) if !raw.is_empty() => raw.split(',').map(str::trim).map(String::from).collect(),
        _ => vec!["llm".into(), "search".into(), "collection".into()],
    };
    let mut items = Vec::new();
    for scope in &scopes {
        items.push(scope_report(&state, scope).await?);
    }
    let all_met = items.iter().all(|item| item["met"] == json!(true));
    Ok(util::ok(&json!({ "met": all_met, "scopes": items })))
}
