//! 公共 API：几个固定地址，把常用操作收成「照着文档就能敲」的形状。
//!
//! 与 `/v1/abilities/{target}/invoke` 的区别只在入口：这里一个地址只做一件事，参数是
//! 普通查询串，返回是可预期的 JSON。内部仍走运维门面的白名单，不绕过任何治理；
//! 投影、查询、加工都复用已有实现，不在这里重写一遍。
//!
//! 写操作只开放两个：跑一条工作流、推进一次加工。信源与适配器的增删改仍留在
//! `/api/ops/...`（控制台）与声明文件上，公共面不提供可以一键删空配置的入口。

use std::sync::Arc;

use ability_support::{ErrorKind, InstanceId, MethodId};
use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use frame_core::{Gateway, InvokeRequest};
use serde::Deserialize;
use serde_json::{Value, json};

/// 列表默认与上限。一次取太多会把公共面变成导出接口。
const DEFAULT_LIMIT: u64 = 20;
const MAX_LIMIT: u64 = 50;

#[derive(Clone)]
struct Api {
    gateway: Arc<dyn Gateway>,
}

pub fn router(gateway: Arc<dyn Gateway>) -> Router {
    let api = Api { gateway };
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/articles", get(articles))
        .route("/api/v1/articles/{rid}", get(article))
        .route("/api/v1/sources", get(sources))
        .route("/api/v1/workflows", get(workflows))
        .route("/api/v1/runs", get(runs))
        .route("/api/v1/workflows/{slug}/run", post(run))
        .route("/api/v1/process", post(process))
        .with_state(api)
}

async fn health(State(api): State<Api>) -> Result<Json<Value>, Failure> {
    let generation = call(&api, "config_admin", "generation", json!({})).await?;
    Ok(Json(json!({
        "status": "ok",
        "service": "modalprism",
        "generation": generation.get("generation").cloned().unwrap_or(Value::Null),
    })))
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    #[serde(default)]
    limit: Option<u64>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    q: Option<String>,
}

impl ListQuery {
    fn limit(&self) -> u64 {
        self.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
    }
}

/// 最新入库的文章。`source` 收 `demo` 或 `source/demo`；`q` 走全文检索。
async fn articles(
    State(api): State<Api>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, Failure> {
    let limit = query.limit();
    let mut spec = json!({
        "from": "resources",
        "hasFacets": [{"kind": "struct", "variant": "article"}],
        "select": [
            "rid",
            "seq",
            "createdAt",
            "sourceId",
            {"kind": "struct", "variant": "article"},
        ],
        "orderBy": [{"field": "seq", "descending": true}],
        "limit": limit,
    });
    if let Some(source) = query
        .source
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        let id = match source.strip_prefix("source/") {
            Some(slug) => format!("source/{slug}"),
            None => format!("source/{source}"),
        };
        spec["where"] = json!({"eq": ["sourceId", id]});
    }
    if let Some(text) = query.q.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        // 检索与 hasFacets 互斥；检索按相关度返回，与按序号分页也互斥。检索命中不限于
        // 文章，下文按结构面再筛一道。
        if let Some(map) = spec.as_object_mut() {
            map.remove("orderBy");
            map.remove("hasFacets");
        }
        spec["search"] = json!({"text": text, "k": limit});
    }
    let page = call(&api, "strata_read", "query", json!({"query": spec})).await?;
    let items: Vec<Value> = page
        .get("rows")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter(|row| is_article(row))
                .map(article_row)
                .collect()
        })
        .unwrap_or_default();
    Ok(Json(json!({
        "items": items,
        "next": page.get("nextCursor").cloned().unwrap_or(Value::Null),
    })))
}

/// 一篇文章的完整投影：结构、可读正文与封面摘要。
async fn article(State(api): State<Api>, Path(rid): Path<String>) -> Result<Json<Value>, Failure> {
    let value = call(&api, "type_article", "project", json!({"rid": rid})).await?;
    if value.is_null() {
        return Err(Failure::missing("文章"));
    }
    Ok(Json(value))
}

async fn sources(State(api): State<Api>) -> Result<Json<Value>, Failure> {
    let value = call(&api, "catalog_admin", "list", json!({})).await?;
    Ok(Json(value))
}

async fn workflows(State(api): State<Api>) -> Result<Json<Value>, Failure> {
    let value = call(&api, "workflow", "catalog", json!({})).await?;
    Ok(Json(json!({
        "workflows": value.get("workflows").cloned().unwrap_or_else(|| json!([])),
        "models": value.get("models").cloned().unwrap_or_else(|| json!([])),
    })))
}

async fn runs(
    State(api): State<Api>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, Failure> {
    let value = call(&api, "workflow", "runs", json!({"limit": query.limit()})).await?;
    Ok(Json(json!({"items": value})))
}

async fn run(
    State(api): State<Api>,
    Path(slug): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, Failure> {
    let rid = body
        .get("rid")
        .and_then(Value::as_str)
        .ok_or_else(|| Failure::invalid("请求体需要 rid"))?;
    let value = call(&api, "workflow", "run", json!({"slug": slug, "rid": rid})).await?;
    Ok(Json(value))
}

/// 推进一次加工：把游标之后的新文章交给启用的工作流。
async fn process(State(api): State<Api>) -> Result<Json<Value>, Failure> {
    let value = call(&api, "workflow", "tick", json!({})).await?;
    Ok(Json(value))
}

async fn call(api: &Api, instance: &str, method: &str, args: Value) -> Result<Value, Failure> {
    let value = raw_call(api, instance, method, args).await?;
    // 公共面保持「照着文档就能敲」的同步语义：长任务句柄在这里轮询到完成。
    let Some(handle) = pending_handle(&value) else {
        return Ok(value);
    };
    let started = std::time::Instant::now();
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let poll = raw_call(api, "committer", "handle", json!({ "handle": handle })).await?;
        match poll.get("status").and_then(Value::as_str) {
            Some("done") => {
                return Ok(poll.get("data").cloned().unwrap_or(Value::Null));
            }
            Some("failed") => {
                let message = poll
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("长任务失败")
                    .to_string();
                return Err(Failure::from(ability_support::Error::failed(message)));
            }
            Some("unknown") => return Err(Failure::missing("句柄")),
            _ if started.elapsed() > std::time::Duration::from_secs(600) => {
                return Err(Failure::from(ability_support::Error::unavailable(
                    "长任务轮询超时",
                )));
            }
            _ => {}
        }
    }
}

async fn raw_call(api: &Api, instance: &str, method: &str, args: Value) -> Result<Value, Failure> {
    let instance =
        InstanceId::new(instance).map_err(|error| Failure::invalid(error.to_string()))?;
    let method = MethodId::new(method).map_err(|error| Failure::invalid(error.to_string()))?;
    api.gateway
        .invoke(InvokeRequest::new(instance, method, args))
        .await
        .map(|response| response.data)
        .map_err(Failure::from)
}

/// 响应是否为待完成的长任务句柄。
fn pending_handle(data: &Value) -> Option<String> {
    if data.get("status").and_then(Value::as_str) == Some("pending") {
        return data
            .get("handle")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    None
}

/// 只保留带文章结构面的资源：检索命中可能来自其它形态。
fn is_article(row: &Value) -> bool {
    row.get("struct:article")
        .is_some_and(|facet| facet.get("value").is_some())
}

/// 查询行里选中的 facet 以 `struct:<variant>` 为键，这里摊平成一层字段。
fn article_row(row: &Value) -> Value {
    let mut out = row
        .get("struct:article")
        .and_then(|facet| facet.get("value"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for key in ["rid", "seq", "createdAt", "sourceId"] {
        if let Some(value) = row.get(key) {
            out.insert(key.to_string(), value.clone());
        }
    }
    Value::Object(out)
}

struct Failure {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl Failure {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "invalid_input",
            message: message.into(),
        }
    }

    fn missing(what: &str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: format!("不存在: {what}"),
        }
    }
}

impl From<ability_support::Error> for Failure {
    fn from(error: ability_support::Error) -> Self {
        let status = match error.kind() {
            ErrorKind::InvalidInput => StatusCode::BAD_REQUEST,
            ErrorKind::NotFound => StatusCode::NOT_FOUND,
            ErrorKind::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::BAD_GATEWAY,
        };
        Self {
            status,
            code: error.code(),
            message: error.message().to_string(),
        }
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let body = json!({"error": {"code": self.code, "message": self.message}});
        (self.status, Json(body)).into_response()
    }
}
