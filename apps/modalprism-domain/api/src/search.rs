//! 内置搜索引擎目录与统一「搜索结果」适配。
//!
//! 密钥与配额在系统设置（settings）；项目只选用引擎配置。引擎返回统一
//! `SearchHit {title, url, snippet, source}`。DDG 为内置免密钥引擎，
//! Bing / Google CSE / SerpAPI 类按 settings 中的密钥引用接入。

use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::response::Response;
use axum::routing::{get, post};
use serde_json::{Value, json};

use crate::Shared;
use crate::error::ApiError;
use crate::util;

const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_HITS: usize = 20;
/// 项目维度搜索记录的文档种类（经 project_binding 关联到项目）。
const SEARCH_RECORD_KIND: &str = "search_record";
/// 搜索引擎平台配置（启用与密钥引用名），与系统设置分开保存。
const PLATFORM_KIND: &str = "search_platform_config";
const PLATFORM_ID: &str = "default";

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/search/engines", get(list_engines))
        .route("/search/engines/test", post(test_engine))
        .route("/search/platforms", get(list_platforms).put(save_platforms))
        .route("/search", post(run_search))
}

/// 引擎目录：`requires_key` 引用 settings 的密钥引用名字段。
/// 每个引擎带图标键与配置字段，前端按同一份目录渲染平台卡片。
pub fn engine_catalog() -> Vec<Value> {
    vec![
        json!({
            "id": "ddg",
            "name": "DuckDuckGo",
            "icon": "ddg",
            "requiresKeyRef": Option::<String>::None,
            "builtin": true,
            "docs": "免密钥，结果受页面标记影响；空结果自动回退 DuckDuckGo Lite",
            "fields": [],
        }),
        json!({
            "id": "ddg_lite",
            "name": "DuckDuckGo Lite",
            "icon": "ddg",
            "requiresKeyRef": Option::<String>::None,
            "builtin": true,
            "docs": "免密钥的轻量页面，标记更稳定",
            "fields": [],
        }),
        json!({
            "id": "bing",
            "name": "Bing Web Search",
            "icon": "bing",
            "requiresKeyRef": "search.bingKeyRef",
            "builtin": false,
            "docs": "密钥引用名指向环境变量，可选 env: 前缀",
            "fields": [{"key": "keyRef", "label": "密钥环境变量名", "required": true, "placeholder": "BING_SEARCH_KEY"}],
        }),
        json!({
            "id": "google_cse",
            "name": "Google Programmable Search (CSE)",
            "icon": "google",
            "requiresKeyRef": "search.googleCseKeyRef",
            "builtin": false,
            "docs": "引用名写成 key,cx（密钥与搜索引擎标识用逗号分隔）",
            "fields": [{"key": "keyRef", "label": "密钥与引擎标识", "required": true, "placeholder": "GOOGLE_KEY,cx"}],
        }),
        json!({
            "id": "serpapi",
            "name": "SerpAPI",
            "icon": "serpapi",
            "requiresKeyRef": "search.serpApiKeyRef",
            "builtin": false,
            "docs": "以 google 引擎参数调用，返回 organic_results",
            "fields": [{"key": "keyRef", "label": "密钥环境变量名", "required": true, "placeholder": "SERPAPI_KEY"}],
        }),
    ]
}

async fn list_engines() -> Result<Response, ApiError> {
    Ok(util::ok(&json!({ "items": engine_catalog() })))
}

/// 平台配置：启用状态与密钥引用名。未保存时回落到系统设置的密钥引用。
async fn platform_items(state: &Shared) -> Vec<Value> {
    let saved = state
        .store
        .get_document(PLATFORM_KIND, PLATFORM_ID)
        .await
        .ok()
        .flatten()
        .map(|doc| doc.body["items"].as_array().cloned().unwrap_or_default())
        .unwrap_or_default();
    let mut items = Vec::new();
    for entry in engine_catalog() {
        let engine = entry["id"].as_str().unwrap_or_default();
        let found = saved
            .iter()
            .find(|item| item["engine"].as_str() == Some(engine));
        let key_field = entry["requiresKeyRef"].as_str().unwrap_or_default();
        let fallback = if key_field.is_empty() {
            String::new()
        } else {
            settings_key_ref(state, key_field).await
        };
        let enabled = found
            .and_then(|item| item["enabled"].as_bool())
            .unwrap_or_else(|| entry["builtin"].as_bool().unwrap_or(false));
        let key_ref = found
            .and_then(|item| item["keyRef"].as_str())
            .map(str::to_string)
            .filter(|value| !value.is_empty())
            .unwrap_or(fallback);
        items.push(json!({
            "engine": engine,
            "enabled": enabled,
            "keyRef": key_ref,
            "catalog": entry,
        }));
    }
    items
}

async fn list_platforms(State(state): State<Shared>) -> Result<Response, ApiError> {
    Ok(util::ok(&json!({"items": platform_items(&state).await})))
}

async fn save_platforms(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["items"])?;
    let items = map
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::validation("items", "required", "需要提供平台列表"))?;
    let known = engine_catalog();
    let mut next = Vec::new();
    for item in items {
        let engine = item["engine"].as_str().unwrap_or_default();
        if !known.iter().any(|entry| entry["id"].as_str() == Some(engine)) {
            return Err(ApiError::validation(
                "items",
                "unknown",
                format!("未知搜索引擎: {engine}"),
            ));
        }
        if !item["enabled"].is_boolean() {
            return Err(ApiError::validation("items", "type", "enabled 必须是布尔值"));
        }
        next.push(json!({
            "engine": engine,
            "enabled": item["enabled"],
            "keyRef": item["keyRef"].as_str().unwrap_or_default(),
        }));
    }
    if !next.iter().any(|item| item["enabled"] == json!(true)) {
        return Err(ApiError::validation(
            "items",
            "required",
            "至少启用一个搜索引擎",
        ));
    }
    let doc = json!({"items": next, "updatedAt": crate::now()});
    match state.store.get_document(PLATFORM_KIND, PLATFORM_ID).await? {
        Some(existing) => {
            state
                .store
                .update_document(PLATFORM_KIND, PLATFORM_ID, &doc, Some(existing.version))
                .await?;
        }
        None => {
            state
                .store
                .insert_document(PLATFORM_KIND, PLATFORM_ID, &doc)
                .await?;
        }
    }
    Ok(util::ok(&json!({"items": platform_items(&state).await})))
}

/// 连通检测：真实跑一次小查询，返回耗时与命中数（不写入搜索记录）。
async fn test_engine(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["engine", "keyRef"])?;
    let engine = util::non_empty(map, "engine")?;
    let key_ref = match util::optional_text(map, "keyRef")? {
        Some(value) if !value.trim().is_empty() => value,
        _ => {
            let field = engine_catalog()
                .iter()
                .find(|entry| entry["id"].as_str() == Some(engine.as_str()))
                .and_then(|entry| entry["requiresKeyRef"].as_str().map(str::to_string))
                .unwrap_or_default();
            if field.is_empty() {
                String::new()
            } else {
                engine_key_ref(&state, &field, &engine).await
            }
        }
    };
    let started = std::time::Instant::now();
    let hits = match engine.as_str() {
        "ddg" => ddg_search("ModalPrism", 3).await?,
        "ddg_lite" => ddg_lite_search("ModalPrism", 3).await?,
        "bing" => bing_search(&key_ref, "ModalPrism", 3).await?,
        "google_cse" => google_cse_search(&key_ref, "ModalPrism", 3).await?,
        "serpapi" => serpapi_search(&key_ref, "ModalPrism", 3).await?,
        other => {
            return Err(ApiError::validation(
                "engine",
                "invalid",
                format!("未知引擎: {other}"),
            ));
        }
    };
    Ok(util::ok(&json!({
        "ok": true,
        "engine": engine,
        "latencyMs": started.elapsed().as_millis() as u64,
        "count": hits.len(),
        "sample": hits.first().cloned().unwrap_or(Value::Null),
    })))
}

fn hit_count(hits: &[Value]) -> usize {
    hits.iter()
        .filter(|hit| hit["url"].as_str().is_some_and(|u| !u.is_empty()))
        .count()
}

/// 统一结果适配：任何引擎结果都归一到 title/url/snippet/source。
fn normalize_hit(hit: Value) -> Value {
    json!({
        "title": hit.get("title").cloned().unwrap_or(Value::Null),
        "url": hit.get("url").cloned().unwrap_or(Value::Null),
        "snippet": hit.get("snippet").cloned().unwrap_or(Value::Null),
        "source": hit.get("source").cloned().unwrap_or(Value::Null),
    })
}

async fn run_search(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["query", "engine", "projectId", "limit"])?;
    let query = util::text(map, "query")?.trim().to_string();
    if query.is_empty() {
        return Err(ApiError::validation("query", "required", "关键词不能为空"));
    }
    let engine = match map.get("engine").and_then(Value::as_str) {
        Some(value) if !value.is_empty() => value.to_string(),
        _ => crate::settings::value(&state, "search.defaultEngine")
            .await
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "ddg".to_string()),
    };
    let limit = map
        .get("limit")
        .and_then(Value::as_u64)
        .map(|value| value.min(MAX_HITS as u64) as usize)
        .unwrap_or(10);

    // projectId 语义：传入则必须指向已存在项目（404），成功后把本次搜索
    // 落为该项目的搜索记录（search_record 绑定），不传行为不变。
    let project_id = match map.get("projectId") {
        Some(Value::String(value)) if !value.trim().is_empty() => value.trim().to_string(),
        Some(_) => {
            return Err(ApiError::validation(
                "projectId",
                "invalid",
                "projectId 必须是非空字符串",
            ));
        }
        None => String::new(),
    };
    if !project_id.is_empty() {
        crate::projects::project_view(&state, &project_id)
            .await
            .map_err(|_| ApiError::not_found(format!("项目 {project_id}")))?;
    }

    // 前置校验：密钥类引擎缺引用名时返回结构化 precondition_failed。
    let catalog = engine_catalog();
    let entry = catalog
        .iter()
        .find(|entry| entry["id"] == json!(engine))
        .ok_or_else(|| ApiError::validation("engine", "invalid", format!("未知引擎: {engine}")))?;
    if let Some(key_ref_field) = entry["requiresKeyRef"].as_str() {
        let key_ref = engine_key_ref(&state, key_ref_field, &engine).await;
        if key_ref.trim().is_empty() {
            return Err(precondition_requirements(vec![json!({
                "type": "engine_key",
                "engine": engine,
                "setting": key_ref_field,
                "message": format!("引擎 {engine} 需要密钥引用名"),
                "where": "设置 → 搜索",
            })]));
        }
    }

    // 密钥引用名来自 settings；实际密钥由各引擎经 resolve_secret 从环境解析。
    // ddg 为主免密钥引擎，其结果被 HTML 标记波动影响：空结果或不可达时自动回退
    // ddg_lite（同为免密钥、标记更稳定）。served_engine 记录真正命中的引擎。
    let mut served_engine = engine.clone();
    let mut degraded = false;
    let hits = match engine.as_str() {
        "ddg" => match ddg_search(&query, limit).await {
            Ok(hits) if !hits.is_empty() => hits,
            result => {
                let primary = result.err().map(|e| e.to_string());
                match ddg_lite_search(&query, limit).await {
                    Ok(hits) if !hits.is_empty() => {
                        served_engine = "ddg_lite".to_string();
                        hits
                    }
                    Ok(_) => {
                        degraded = true;
                        if let Some(error) = primary {
                            return Err(ApiError::dependency(format!(
                                "搜索引擎未能返回结果（ddg: {error}；ddg_lite 亦为空）"
                            ))
                            .retryable());
                        }
                        Vec::new()
                    }
                    Err(error) => {
                        // ddg 与 ddg_lite 都不可达：回传 lite 侧错误（更可能是根因）。
                        let _ = primary;
                        return Err(error);
                    }
                }
            }
        },
        "ddg_lite" => ddg_lite_search(&query, limit).await?,
        "bing" => {
            let key_ref = engine_key_ref(&state, "search.bingKeyRef", "bing").await;
            bing_search(&key_ref, &query, limit).await?
        }
        "google_cse" => {
            let key_ref = engine_key_ref(&state, "search.googleCseKeyRef", "google_cse").await;
            google_cse_search(&key_ref, &query, limit).await?
        }
        "serpapi" => {
            let key_ref = engine_key_ref(&state, "search.serpApiKeyRef", "serpapi").await;
            serpapi_search(&key_ref, &query, limit).await?
        }
        _ => unreachable!("引擎已校验"),
    };
    let hits: Vec<Value> = hits.into_iter().take(limit).map(normalize_hit).collect();
    if hits.is_empty() {
        degraded = true;
    }

    // 最小关联：项目维度搜索落一条 search_record 并绑定到项目，
    // 可经 /projects/{projectId}/resources?kind=search_record 查看。
    if !project_id.is_empty() {
        let record_id = crate::new_id("srch");
        let urls: Vec<Value> = hits
            .iter()
            .filter_map(|hit| hit["url"].as_str())
            .take(5)
            .map(|url| json!(url))
            .collect();
        let record = json!({
            "id": record_id,
            "projectId": project_id,
            "engine": served_engine,
            "requestedEngine": engine,
            "query": query,
            "count": hit_count(&hits),
            "topUrls": urls,
            "createdAt": crate::now(),
        });
        state
            .store
            .insert_document(SEARCH_RECORD_KIND, &record_id, &record)
            .await?;
        state
            .store
            .insert_document(
                crate::projects::BINDING_KIND,
                &format!("{SEARCH_RECORD_KIND}:{record_id}"),
                &json!({
                    "projectId": project_id,
                    "resourceKind": SEARCH_RECORD_KIND,
                    "resourceId": record_id,
                    "boundAt": crate::now(),
                }),
            )
            .await?;
    }

    let mut payload = json!({
        "engine": served_engine,
        "query": query,
        "count": hit_count(&hits),
        "results": hits,
    });
    if served_engine != engine {
        payload["requestedEngine"] = json!(engine);
    }
    if degraded {
        payload["degraded"] = json!(true);
        payload["note"] = json!(
            "免密钥引擎未能返回结果（DDG 主页与 Lite 均为空）；如需稳定结果请在设置配置带密钥引擎。"
        );
    }
    if !project_id.is_empty() {
        payload["projectId"] = json!(project_id);
    }
    Ok(util::ok(&payload))
}

/// 结构化 412 前置失败：缺什么、去哪配。裸 409 不够，用同一错误码约定。
pub fn precondition_requirements(requirements: Vec<Value>) -> ApiError {
    let summary = requirements
        .iter()
        .filter_map(|item| item["message"].as_str())
        .collect::<Vec<_>>()
        .join("; ");
    let mut error = ApiError::precondition(if summary.is_empty() {
        "前置条件未满足"
    } else {
        &summary
    });
    // 把结构化需求挂到 fieldErrors 之外：借用 field_errors 表达「去哪配」。
    for item in &requirements {
        let field = item["setting"]
            .as_str()
            .or(item["engine"].as_str())
            .unwrap_or("precondition");
        error.field_errors.push(crate::error::FieldError {
            field: field.to_string(),
            code: item["type"].as_str().unwrap_or("precondition").to_string(),
            message: item["message"].as_str().unwrap_or_default().to_string(),
        });
    }
    error
}

/// 从 settings 读取某引擎的密钥引用名（未配置时为空串，由 resolve_secret 报 412）。
async fn settings_key_ref(state: &Shared, field: &str) -> String {
    crate::settings::value(state, field)
        .await
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// 引擎密钥引用：平台配置优先，其次回落系统设置；空串表示未配置。
async fn engine_key_ref(state: &Shared, field: &str, engine: &str) -> String {
    if let Ok(Some(doc)) = state.store.get_document(PLATFORM_KIND, PLATFORM_ID).await
        && let Some(value) = doc.body["items"]
            .as_array()
            .and_then(|items| {
                items
                    .iter()
                    .find(|item| item["engine"].as_str() == Some(engine))
            })
            .and_then(|item| item["keyRef"].as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
    {
        return value.to_string();
    }
    settings_key_ref(state, field).await
}

/// 解析密钥引用名：`env:VAR` 或裸名字都指向环境变量；`value:literal` 直接取字面量。
/// 引用名为空或环境变量缺失时返回结构化 412（缺什么、去哪配）。
pub fn resolve_secret(ref_setting: &str, ref_name: &str) -> Result<String, ApiError> {
    let name = ref_name.trim();
    if name.is_empty() {
        return Err(precondition_requirements(vec![json!({
            "type": "engine_key",
            "setting": ref_setting,
            "message": format!("设置 {ref_setting} 未配置密钥引用名"),
            "where": "设置 → 搜索",
        })]));
    }
    let secret = if let Some(literal) = name.strip_prefix("value:") {
        literal.to_string()
    } else {
        let var = name.strip_prefix("env:").unwrap_or(name);
        std::env::var(var).unwrap_or_default()
    };
    if secret.trim().is_empty() {
        return Err(precondition_requirements(vec![json!({
            "type": "engine_key",
            "setting": ref_setting,
            "message": format!("密钥引用 {name} 在环境中不可读或为空"),
            "where": "设置 → 搜索（检查宿主环境变量）",
        })]));
    }
    Ok(secret.trim().to_string())
}

/// 测试/内网用端点覆盖：环境变量 `MODALPRISM_SEARCH_ENDPOINT_<ENGINE>`，
/// 或测试代码经 `set_endpoint_override` 注入（工作区禁 unsafe，不能在测试里改环境）。
static ENDPOINT_OVERRIDES: std::sync::OnceLock<
    std::sync::Mutex<std::collections::BTreeMap<String, String>>,
> = std::sync::OnceLock::new();

pub fn set_endpoint_override(engine: &str, url: &str) {
    let map = ENDPOINT_OVERRIDES.get_or_init(|| std::sync::Mutex::new(Default::default()));
    map.lock()
        .unwrap()
        .insert(engine.to_string(), url.to_string());
}

fn endpoint_for(engine: &str, default: &str) -> String {
    if let Some(map) = ENDPOINT_OVERRIDES.get() {
        if let Some(url) = map.lock().unwrap().get(engine) {
            return url.clone();
        }
    }
    std::env::var(format!(
        "MODALPRISM_SEARCH_ENDPOINT_{}",
        engine.to_uppercase()
    ))
    .ok()
    .filter(|value| !value.trim().is_empty())
    .unwrap_or_else(|| default.to_string())
}

fn search_client() -> Result<reqwest::Client, ApiError> {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|error| ApiError::dependency(format!("搜索客户端不可用: {error}")))
}

async fn read_json(response: reqwest::Response, what: &str) -> Result<Value, ApiError> {
    let status = response.status();
    if !status.is_success() {
        let detail = response.text().await.unwrap_or_default();
        let tail: String = detail.chars().take(200).collect();
        return Err(ApiError::dependency(format!("{what} 返回 {status}: {tail}")).retryable());
    }
    let value: Value = response
        .json()
        .await
        .map_err(|error| ApiError::dependency(format!("{what} 响应解析失败: {error}")))?;
    Ok(value)
}

/// Bing Web Search v7：`Ocp-Apim-Subscription-Key` 头 + JSON webPages。
pub async fn bing_search(key_ref: &str, query: &str, limit: usize) -> Result<Vec<Value>, ApiError> {
    let key = resolve_secret("search.bingKeyRef", key_ref)?;
    let url = format!(
        "{}?q={}",
        endpoint_for("bing", "https://api.bing.microsoft.com/v7.0/search"),
        urlencoded(query)
    );
    let response = search_client()?
        .get(&url)
        .header("Ocp-Apim-Subscription-Key", key)
        .send()
        .await
        .map_err(|error| ApiError::dependency(format!("Bing 不可达: {error}")).retryable())?;
    let body = read_json(response, "Bing").await?;
    Ok(body["webPages"]["value"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .take(limit)
                .map(|item| {
                    json!({
                        "title": item["name"].clone(),
                        "url": item["url"].clone(),
                        "snippet": item["snippet"].clone(),
                        "source": "bing",
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Google Programmable Search：引用名 `key,cx`（逗号分隔）。
pub async fn google_cse_search(
    key_ref: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<Value>, ApiError> {
    let secret = resolve_secret("search.googleCseKeyRef", key_ref)?;
    let (key, cx) = match secret.split_once(',') {
        Some((key, cx)) => (key.trim().to_string(), cx.trim().to_string()),
        None => {
            return Err(precondition_requirements(vec![json!({
                "type": "engine_key",
                "setting": "search.googleCseKeyRef",
                "message": "Google CSE 引用名需以 key,cx 形式提供",
                "where": "设置 → 搜索",
            })]));
        }
    };
    let url = format!(
        "{}?key={}&cx={}&q={}",
        endpoint_for("google_cse", "https://www.googleapis.com/customsearch/v1"),
        urlencoded(&key),
        urlencoded(&cx),
        urlencoded(query)
    );
    let response =
        search_client()?.get(&url).send().await.map_err(|error| {
            ApiError::dependency(format!("Google CSE 不可达: {error}")).retryable()
        })?;
    let body = read_json(response, "Google CSE").await?;
    Ok(body["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .take(limit)
                .map(|item| {
                    json!({
                        "title": item["title"].clone(),
                        "url": item["link"].clone(),
                        "snippet": item["snippet"].clone(),
                        "source": "google_cse",
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

/// SerpAPI：`api_key` 参数 + organic_results。
pub async fn serpapi_search(
    key_ref: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<Value>, ApiError> {
    let key = resolve_secret("search.serpApiKeyRef", key_ref)?;
    let url = format!(
        "{}?engine=google&q={}&api_key={}",
        endpoint_for("serpapi", "https://serpapi.com/search"),
        urlencoded(query),
        urlencoded(&key)
    );
    let response =
        search_client()?.get(&url).send().await.map_err(|error| {
            ApiError::dependency(format!("SerpAPI 不可达: {error}")).retryable()
        })?;
    let body = read_json(response, "SerpAPI").await?;
    Ok(body["organic_results"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .take(limit)
                .map(|item| {
                    json!({
                        "title": item["title"].clone(),
                        "url": item["link"].clone(),
                        "snippet": item["snippet"].clone(),
                        "source": "serpapi",
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

/// 查询串编码（空格转 +，保留非 ASCII 的 UTF-8 百分号编码）。
fn urlencoded(input: &str) -> String {
    let mut out = String::new();
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            byte => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// DDG HTML 端点搜索：POST form，解析结果链接与摘要。
async fn ddg_search(query: &str, limit: usize) -> Result<Vec<Value>, ApiError> {
    let client = reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|error| ApiError::dependency(format!("搜索客户端不可用: {error}")))?;
    let response = client
        .post("https://html.duckduckgo.com/html/")
        .form(&[("q", query)])
        .header("user-agent", "Mozilla/5.0 (compatible; ModalPrism)")
        .send()
        .await
        .map_err(|error| ApiError::dependency(format!("搜索引擎不可达: {error}")).retryable())?;
    let status = response.status();
    if !status.is_success() {
        return Err(ApiError::dependency(format!("搜索引擎返回 {status}")).retryable());
    }
    let html = response
        .text()
        .await
        .map_err(|error| ApiError::dependency(format!("搜索响应读取失败: {error}")))?;
    Ok(parse_ddg_html(&html, limit))
}

/// DDG Lite 端点搜索：HTML 主页标记变化时的免密钥回退路径。
async fn ddg_lite_search(query: &str, limit: usize) -> Result<Vec<Value>, ApiError> {
    let client = reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|error| ApiError::dependency(format!("搜索客户端不可用: {error}")))?;
    let response = client
        .post("https://lite.duckduckgo.com/lite/")
        .form(&[("q", query)])
        .header("user-agent", "Mozilla/5.0 (compatible; ModalPrism)")
        .send()
        .await
        .map_err(|error| ApiError::dependency(format!("搜索引擎不可达: {error}")).retryable())?;
    let status = response.status();
    if !status.is_success() {
        return Err(ApiError::dependency(format!("搜索引擎返回 {status}")).retryable());
    }
    let html = response
        .text()
        .await
        .map_err(|error| ApiError::dependency(format!("搜索响应读取失败: {error}")))?;
    Ok(parse_ddg_lite_html(&html, limit))
}

/// 从 DDG Lite 结果页提取统一搜索结果。纯函数，便于离线测试。
/// Lite 版为表格布局：`<a href="URL" class='result-link'>标题</a>`，
/// href 在标记之前、标题在标记之后，故按位置扫描而非整块切分。
pub fn parse_ddg_lite_html(html: &str, limit: usize) -> Vec<Value> {
    let mut hits = Vec::new();
    let marker = "result-link";
    let mut from = 0usize;
    while hits.len() < limit {
        let Some(rel) = html[from..].find(marker) else {
            break;
        };
        let anchor_at = from + rel;
        // 标题：跳过 `<a …>` 开标签剩余，取到 `</a>`。
        let after_marker = anchor_at + marker.len();
        let Some(open_gt) = html[after_marker..].find('>') else {
            from = after_marker;
            continue;
        };
        let title_start = after_marker + open_gt + 1;
        let Some(title_rel_end) = html[title_start..].find("</a>") else {
            from = after_marker;
            continue;
        };
        let title = strip_tags(&html[title_start..title_start + title_rel_end]);
        // 链接：标记之前最近的 href（属于本 `<a>`）。
        let Some(url) = extract_href(&html[..anchor_at]) else {
            from = title_start;
            continue;
        };
        let url = decode_ddg_redirect(url.trim());
        if url.is_empty() {
            from = title_start;
            continue;
        }
        // 摘要：标题之后第一个 result-snippet 单元。
        let body_from = title_start + title_rel_end;
        let snippet = html[body_from..]
            .find("result-snippet")
            .and_then(|rel_start| {
                let block = &html[body_from + rel_start..];
                let gt = block.find('>')?;
                let rest = &block[gt + 1..];
                let end = rest.find("</td>")?;
                Some(strip_tags(&rest[..end]))
            })
            .unwrap_or_default();
        hits.push(json!({
            "title": title,
            "url": url,
            "snippet": snippet,
            "source": "ddg_lite",
        }));
        from = body_from;
    }
    hits
}

/// 从标签片段里取最后一个 `href="…"` 或 `href='…'`。
fn extract_href(fragment: &str) -> Option<String> {
    let rest = fragment.rsplit("href=").next()?;
    let trimmed = rest.trim_start();
    let quote = trimmed.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let value = &trimmed[1..];
    value.split(quote).next().map(str::to_string)
}

/// 从 DDG HTML 端点结果页提取统一搜索结果。纯函数，便于离线测试。
/// 按 `result__body` 子串切块：兼容旧版 `class="result__body"` 与现网
/// `class="links_main links_deep result__body"`（前缀类名会变，子串稳定）。
pub fn parse_ddg_html(html: &str, limit: usize) -> Vec<Value> {
    let mut hits = Vec::new();
    for segment in html.split("result__body").skip(1) {
        if hits.len() >= limit {
            break;
        }
        let title = extract_anchor(segment, "result__a");
        let Some((url, title_text)) = title else {
            continue;
        };
        let url = decode_ddg_redirect(&url);
        let snippet = segment
            .split("class=\"result__snippet\"")
            .nth(1)
            .and_then(|part| {
                let start = part.find('>')?;
                let rest = &part[start + 1..];
                let end = rest.find("</a>")?;
                Some(strip_tags(&rest[..end]))
            })
            .unwrap_or_default();
        hits.push(json!({
            "title": title_text,
            "url": url,
            "snippet": snippet,
            "source": "ddg",
        }));
    }
    hits
}

/// 抽取 `<a class="...">…</a>`，返回 (href, 文本)。
fn extract_anchor(segment: &str, class: &str) -> Option<(String, String)> {
    let anchor_start = segment.find(&format!("class=\"{class}\""))?;
    let tag_start = segment[..anchor_start].rfind("<a ")?;
    let rest = &segment[tag_start..];
    let tag_end = rest.find("</a>")?;
    let tag = &rest[..tag_end];
    let href = tag.split("href=\"").nth(1)?.split('"').next()?.to_string();
    let inner = &rest[tag.find('>')? + 1..tag_end];
    Some((href, strip_tags(inner)))
}

/// DDG 链接常是 `/l/?uddg=<encoded>` 形式的跳转；解出真实地址。
fn decode_ddg_redirect(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("/l/?") {
        for pair in rest.split('&') {
            if let Some(value) = pair.strip_prefix("uddg=") {
                return urldecode(value);
            }
        }
    }
    url.to_string()
}

fn urldecode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    index += 3;
                    continue;
                }
                out.push(bytes[index]);
                index += 1;
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 去掉 HTML 标签并压缩空白，得到纯文本。
fn strip_tags(input: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            ch if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ddg_html_extracts_unified_hits() {
        let html = r##"
        <div class="result results_links">
          <div class="result__body">
            <h2 class="result__title"><a class="result__a" href="/l/?uddg=https%3A%2F%2Fexample.com%2Fa">Example A</a></h2>
            <a class="result__snippet" href="#">First <b>snippet</b> here</a>
          </div>
          <div class="result__body">
            <h2 class="result__title"><a class="result__a" href="https://example.org/b">Example B</a></h2>
            <a class="result__snippet" href="#">Second snippet</a>
          </div>
        </div>"##;
        let hits = parse_ddg_html(html, 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0]["url"], "https://example.com/a");
        assert_eq!(hits[0]["title"], "Example A");
        assert_eq!(hits[0]["snippet"], "First snippet here");
        assert_eq!(hits[1]["url"], "https://example.org/b");
        for hit in &hits {
            let normalized = normalize_hit(hit.clone());
            assert!(normalized.get("title").is_some());
            assert!(normalized.get("source").is_some());
        }
    }

    #[test]
    fn parse_ddg_html_respects_limit_and_empty() {
        assert!(parse_ddg_html("<html></html>", 10).is_empty());
        let html = r#"
        <div class="result__body"><a class="result__a" href="https://a">A</a></div>
        <div class="result__body"><a class="result__a" href="https://b">B</a></div>"#;
        assert_eq!(parse_ddg_html(html, 1).len(), 1);
    }

    // 现网 DDG 把结果块类名改成 links_main links_deep result__body，
    // 解析须按 result__body 子串切块仍能命中。
    #[test]
    fn parse_ddg_html_handles_current_markup() {
        let html = r##"
        <div class="result results_links results_links_deep web-result ">
          <div class="links_main links_deep result__body">
            <h2 class="result__title">
              <a rel="nofollow" class="result__a" href="https://rust-lang.org/">Rust Programming Language</a>
            </h2>
            <a class="result__snippet" href="https://rust-lang.org/"><b>Rust</b> is a fast, reliable language.</a>
          </div>
        </div>"##;
        let hits = parse_ddg_html(html, 10);
        assert_eq!(hits.len(), 1, "现网标记应解析出 1 条");
        assert_eq!(hits[0]["url"], "https://rust-lang.org/");
        assert_eq!(hits[0]["title"], "Rust Programming Language");
    }

    #[test]
    fn parse_ddg_lite_html_extracts_table_hits() {
        let html = r#"
        <table>
          <tr><td><a rel="nofollow" href="https://rust-lang.org/" class='result-link'>Rust Programming Language</a></td></tr>
          <tr><td>&nbsp;</td><td class='result-snippet'><b>Rust</b> is a fast, reliable language.</td></tr>
          <tr><td><a rel="nofollow" href="https://doc.rust-lang.org/" class='result-link'>The Rust Programming Language</a></td></tr>
          <tr><td>&nbsp;</td><td class='result-snippet'>Book about Rust.</td></tr>
        </table>"#;
        let hits = parse_ddg_lite_html(html, 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0]["url"], "https://rust-lang.org/");
        assert_eq!(hits[0]["title"], "Rust Programming Language");
        assert_eq!(hits[0]["snippet"], "Rust is a fast, reliable language.");
        assert_eq!(hits[0]["source"], "ddg_lite");
        assert_eq!(hits[1]["url"], "https://doc.rust-lang.org/");
    }

    #[test]
    fn engine_catalog_lists_builtin_without_key() {
        let ddg = engine_catalog()
            .into_iter()
            .find(|entry| entry["id"] == "ddg")
            .expect("ddg entry");
        assert!(ddg["requiresKeyRef"].is_null());
    }

    #[test]
    fn resolve_secret_reads_literal_and_rejects_missing() {
        // value: 字面量与缺失路径不需要环境写入（工作区禁 unsafe）。
        assert_eq!(
            resolve_secret("search.bingKeyRef", "value:abc").unwrap(),
            "abc"
        );
        let missing = resolve_secret("search.bingKeyRef", "env:MODALPRISM_NOPE").unwrap_err();
        assert_eq!(missing.status, axum::http::StatusCode::PRECONDITION_FAILED);
        assert_eq!(missing.field_errors[0].code, "engine_key");
        let empty = resolve_secret("search.bingKeyRef", "").unwrap_err();
        assert_eq!(empty.status, axum::http::StatusCode::PRECONDITION_FAILED);
    }

    #[test]
    fn urlencoded_encodes_query_strings() {
        assert_eq!(urlencoded("hello world"), "hello+world");
        assert_eq!(urlencoded("a&b=c"), "a%26b%3Dc");
        assert_eq!(urlencoded("测试"), "%E6%B5%8B%E8%AF%95");
    }

    fn bing_json() -> Value {
        json!({
            "webPages": {"value": [
                {"name": "Bing 标题", "url": "https://example.com/1", "snippet": "摘要一"},
                {"name": "第二条", "url": "https://example.org/2", "snippet": "摘要二"}
            ]}
        })
    }

    #[test]
    fn bing_hits_map_to_unified_shape() {
        let body = bing_json();
        let hits: Vec<Value> = body["webPages"]["value"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| {
                json!({
                    "title": item["name"].clone(),
                    "url": item["url"].clone(),
                    "snippet": item["snippet"].clone(),
                    "source": "bing",
                })
            })
            .collect();
        assert_eq!(hits[0]["title"], json!("Bing 标题"));
        assert_eq!(hits[0]["source"], json!("bing"));
        let normalized = normalize_hit(hits[0].clone());
        assert_eq!(normalized["url"], json!("https://example.com/1"));
    }

    #[test]
    fn cse_and_serpapi_hits_map_to_unified_shape() {
        let cse_items = json!([
            {"title": "CSE 标题", "link": "https://c.example/1", "snippet": "c 摘要"}
        ]);
        let hit = &cse_items[0];
        let normalized = normalize_hit(json!({
            "title": hit["title"], "url": hit["link"], "snippet": hit["snippet"], "source": "google_cse",
        }));
        assert_eq!(normalized["url"], json!("https://c.example/1"));
        assert_eq!(normalized["source"], json!("google_cse"));

        let serp_items = json!([
            {"title": "Serp 标题", "link": "https://s.example/1", "snippet": "s 摘要"}
        ]);
        let normalized = normalize_hit(json!({
            "title": serp_items[0]["title"], "url": serp_items[0]["link"],
            "snippet": serp_items[0]["snippet"], "source": "serpapi",
        }));
        assert_eq!(normalized["url"], json!("https://s.example/1"));
        assert_eq!(normalized["source"], json!("serpapi"));
    }

    // 真网门控：设 MODALPRISM_LIVE_SEARCH=1 且相应密钥环境变量在位时，
    // 用 `cargo test -p domain-api -- --ignored` 显式运行，不进默认 CI 全绿路径。
    #[tokio::test]
    #[ignore = "真网测试：需 MODALPRISM_LIVE_SEARCH=1 与真实密钥"]
    async fn live_bing_search() {
        live_search("bing").await;
    }

    #[tokio::test]
    #[ignore = "真网测试：需 MODALPRISM_LIVE_SEARCH=1 与真实密钥（key,cx）"]
    async fn live_google_cse_search() {
        live_search("google_cse").await;
    }

    #[tokio::test]
    #[ignore = "真网测试：需 MODALPRISM_LIVE_SEARCH=1 与真实密钥"]
    async fn live_serpapi_search() {
        live_search("serpapi").await;
    }

    async fn live_search(engine: &str) {
        if std::env::var("MODALPRISM_LIVE_SEARCH").ok().as_deref() != Some("1") {
            panic!("真网测试未启用：设置 MODALPRISM_LIVE_SEARCH=1 后重跑");
        }
        // 密钥引用直接指向环境变量 MODALPRISM_LIVE_<engine>；缺失时 resolve_secret 会 412。
        let ref_name = format!("env:MODALPRISM_LIVE_{engine}");
        assert!(
            std::env::var(format!("MODALPRISM_LIVE_{engine}")).is_ok(),
            "缺少真网密钥环境变量 MODALPRISM_LIVE_{engine}"
        );
        let hits = match engine {
            "bing" => bing_search(&ref_name, "modalprism", 5).await,
            "google_cse" => google_cse_search(&ref_name, "modalprism", 5).await,
            _ => serpapi_search(&ref_name, "modalprism", 5).await,
        };
        let hits = hits.expect("真网搜索应成功");
        assert!(!hits.is_empty(), "真网搜索应至少返回一条结果");
        assert!(hits[0]["url"].as_str().unwrap().starts_with("http"));
    }
}
