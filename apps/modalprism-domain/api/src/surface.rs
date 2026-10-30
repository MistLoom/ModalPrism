//! 友好对外表面：`POST /fetch`（URL → 正文）与 `POST /parse`（文本/URL →
//! 结构化抽取）。fetch 管获取，rawprism 管结构化抽取，二者解耦。

use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use serde_json::{Value, json};

use crate::Shared;
use crate::error::ApiError;
use crate::util;

const FETCH_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/fetch", post(fetch_url))
        .route("/parse", post(parse_input))
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FetchBody {
    url: String,
    #[serde(default)]
    max_bytes: Option<u64>,
}

/// `POST /fetch`：URL → 正文。回环内自动透传（遵循本机回环约束）。
async fn fetch_url(
    State(_state): State<Shared>,
    Json(body): Json<FetchBody>,
) -> Result<Response, ApiError> {
    let url = body.url.trim();
    let parsed =
        url::Url::parse(url).map_err(|_| ApiError::validation("url", "invalid", "URL 无法解析"))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(ApiError::validation("url", "invalid", "仅支持 http/https"));
    }
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .map_err(|error| ApiError::dependency(format!("HTTP 客户端不可用: {error}")))?;
    let response = client
        .get(parsed.clone())
        .header("user-agent", "Mozilla/5.0 (compatible; ModalPrism)")
        .send()
        .await
        .map_err(|error| ApiError::dependency(format!("获取失败: {error}")).retryable())?;
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let limit = body
        .max_bytes
        .unwrap_or(MAX_BODY_BYTES as u64)
        .min(MAX_BODY_BYTES as u64);
    let text = response
        .bytes()
        .await
        .map_err(|error| ApiError::dependency(format!("读取响应失败: {error}")))?
        .iter()
        .take(limit as usize)
        .cloned()
        .collect::<Vec<u8>>();
    let text = String::from_utf8_lossy(&text).into_owned();
    Ok(util::ok(&json!({
        "url": parsed.as_str(),
        "status": status,
        "contentType": content_type,
        "truncated": text.len() as u64 >= limit,
        "text": text,
    })))
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ParseBody {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    selectors: Option<Vec<String>>,
    #[serde(default)]
    config: Option<Value>,
}

/// `POST /parse`：文本或 URL → rawprism 结构化抽取（类型计数 + 区域）。
async fn parse_input(
    State(state): State<Shared>,
    Json(body): Json<ParseBody>,
) -> Result<Response, ApiError> {
    let text = if let Some(url) = body.url.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        let fetch: Value = serde_json::from_value(json!({ "url": url }))
            .map_err(|error| ApiError::internal(format!("请求构造失败: {error}")))?;
        let _ = fetch;
        fetch_text(url).await?
    } else {
        body.text
            .clone()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ApiError::validation("text|url", "required", "text 与 url 至少提供一个")
            })?
    };
    let hook = state.parse_hook.get().cloned().ok_or_else(|| {
        crate::search::precondition_requirements(vec![json!({
            "type": "parser",
            "message": "结构化解析能力（rawprism）未接入",
            "where": "采集侧加载 rawprism 能力",
        })])
    })?;
    let mut args = json!({ "text": text });
    if let Some(selectors) = &body.selectors {
        args["selectors"] = json!(selectors);
    }
    if let Some(config) = &body.config {
        args["config"] = config.clone();
    }
    let result = hook(&args)
        .await
        .map_err(|error| ApiError::dependency(error).retryable())?;
    Ok(util::ok(&result))
}

async fn fetch_text(url: &str) -> Result<String, ApiError> {
    let parsed =
        url::Url::parse(url).map_err(|_| ApiError::validation("url", "invalid", "URL 无法解析"))?;
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .map_err(|error| ApiError::dependency(format!("HTTP 客户端不可用: {error}")))?;
    let response = client
        .get(parsed)
        .header("user-agent", "Mozilla/5.0 (compatible; ModalPrism)")
        .send()
        .await
        .map_err(|error| ApiError::dependency(format!("获取失败: {error}")).retryable())?;
    let bytes = response
        .bytes()
        .await
        .map_err(|error| ApiError::dependency(format!("读取响应失败: {error}")))?
        .iter()
        .take(MAX_BODY_BYTES)
        .cloned()
        .collect::<Vec<u8>>();
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
