//! 内容投递：手动投递、自动投递规则、外部 API 目标与投递记录。
//!
//! 与「通知推送」分工：这里投递的是**内容**（标题、摘要、正文），目标是模型、
//! 外部 API 或通知渠道；结果只保留记录与明细，供人工核对。
//!
//! 自动投递以「订阅」方式实现：规则声明监听对象（信源或计划）与目标，
//! 后台按 `lastRunAt` 增量扫描新入库内容并投递，不改动采集链路。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{get, patch, post};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use strata_model::{Expr, OrderBy, Query as StrataQuery, QueryFrom, SelectItem};
use tokio::sync::OnceCell;

use crate::error::ApiError;
use crate::operations::accepted;
use crate::util::{self, cursor, cursor_offset, page};
use crate::{ActiveProject, AppState, Shared, new_id, now};

const RECORD_KIND: &str = "delivery_record";
const RULE_KIND: &str = "delivery_rule";
const ENDPOINT_KIND: &str = "delivery_endpoint";
/// 单次自动投递批量上限：避免一次规则轮询把队列打满。
const AUTO_BATCH: usize = 20;

static SWEEPER_BUSY: AtomicBool = AtomicBool::new(false);
static SWEEPER: OnceCell<()> = OnceCell::const_new();

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/delivery/targets", get(list_targets))
        .route("/delivery/endpoints", get(list_endpoints).post(create_endpoint))
        .route(
            "/delivery/endpoints/{endpointId}",
            patch(update_endpoint).delete(delete_endpoint),
        )
        .route("/delivery/rules", get(list_rules).post(create_rule))
        .route(
            "/delivery/rules/{ruleId}",
            patch(update_rule).delete(delete_rule),
        )
        .route("/delivery/records", get(list_records))
        .route("/delivery/records/{recordId}", get(get_record))
        .route("/delivery/jobs", post(create_job))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListQuery {
    #[serde(default)]
    limit: Option<String>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

fn with_etag(mut body: Value, kind: &str, id: &str, version: i64) -> Value {
    if let Some(map) = body.as_object_mut() {
        map.insert("etag".into(), Value::String(format!("{kind}-{id}-v{version}")));
    }
    body
}

async fn page_docs(
    state: &AppState,
    kind: &str,
    query: ListQuery,
    filter: impl Fn(&Value) -> bool,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let tag = format!(
        "{kind}:{}:{}",
        query.kind.clone().unwrap_or_default(),
        query.status.clone().unwrap_or_default()
    );
    let offset = cursor_offset(query.cursor.as_deref(), &tag)?;
    let mut docs: Vec<Value> = state
        .store
        .list_documents(kind)
        .await?
        .into_iter()
        .filter(|doc| filter(&doc.body))
        .map(|doc| with_etag(doc.body, kind, &doc.id, doc.version))
        .collect();
    docs.sort_by(|left, right| {
        right["createdAt"]
            .as_str()
            .unwrap_or("")
            .cmp(left["createdAt"].as_str().unwrap_or(""))
    });
    let total = docs.len();
    let items: Vec<Value> = docs.into_iter().skip(offset).take(limit + 1).collect();
    let more = items.len() > limit;
    let items: Vec<Value> = items.into_iter().take(limit).collect();
    let next = if more || offset + items.len() < total {
        Some(cursor(&tag, offset + limit))
    } else {
        None
    };
    Ok(util::ok(&page(items, next)))
}

/// 目标目录：模型、外部 API 与通知渠道。前端只渲染，不做能力推断。
async fn list_targets(State(state): State<Shared>) -> Result<Response, ApiError> {
    let channels = state
        .store
        .list_documents("channel")
        .await?
        .into_iter()
        .map(|doc| {
            json!({
                "kind": "channel",
                "id": doc.id,
                "name": doc.body["name"],
                "detail": doc.body["kind"],
                "enabled": doc.body["enabled"],
            })
        })
        .collect::<Vec<_>>();
    let models = state
        .store
        .list_documents("model")
        .await?
        .into_iter()
        .map(|doc| {
            json!({
                "kind": "model",
                "id": doc.id,
                "name": doc.body["name"],
                "detail": doc.body["kind"],
                "enabled": doc.body["enabled"],
            })
        })
        .collect::<Vec<_>>();
    let endpoints = state
        .store
        .list_documents(ENDPOINT_KIND)
        .await?
        .into_iter()
        .map(|doc| {
            json!({
                "kind": "endpoint",
                "id": doc.id,
                "name": doc.body["name"],
                "detail": doc.body["url"],
                "enabled": doc.body["enabled"],
            })
        })
        .collect::<Vec<_>>();
    let all: Vec<Value> = channels
        .iter()
        .chain(models.iter())
        .chain(endpoints.iter())
        .cloned()
        .collect();
    Ok(util::ok(&json!({
        "items": all,
        "groups": [
            {"kind": "model", "label": "模型", "note": "交给已登记模型处理，结果留在投递记录里"},
            {"kind": "endpoint", "label": "外部 API", "note": "按 JSON 提交到自定义地址"},
            {"kind": "channel", "label": "通知渠道", "note": "投递到已有的 Webhook 或飞书渠道"},
        ],
    })))
}

async fn list_endpoints(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    page_docs(&state, ENDPOINT_KIND, query, |_| true).await
}

fn endpoint_fields(map: &Map<String, Value>) -> Result<Value, ApiError> {
    let name = util::non_empty(map, "name")?;
    let url = util::non_empty(map, "url")?;
    let parsed = url::Url::parse(&url)
        .map_err(|_| ApiError::validation("url", "format", "不是有效地址"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(ApiError::validation("url", "scheme", "只接受 http/https"));
    }
    let token_ref = util::optional_text(map, "tokenRef")?
        .unwrap_or_default()
        .trim()
        .to_string();
    let enabled = util::optional_flag(map, "enabled", true)?;
    Ok(json!({
        "name": name,
        "url": url,
        "tokenRef": token_ref,
        "enabled": enabled,
    }))
}

async fn create_endpoint(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["name", "url", "tokenRef", "enabled"])?;
    let fields = endpoint_fields(map)?;
    let id = new_id("endpoint");
    let mut doc = fields;
    doc["id"] = Value::String(id.clone());
    doc["createdAt"] = Value::String(now());
    doc["updatedAt"] = Value::String(now());
    state.store.insert_document(ENDPOINT_KIND, &id, &doc).await?;
    Ok(util::created(&doc))
}

async fn update_endpoint(
    State(state): State<Shared>,
    Path(endpoint_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["name", "url", "tokenRef", "enabled"])?;
    let doc = state
        .store
        .get_document(ENDPOINT_KIND, &endpoint_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("外部 API {endpoint_id}")))?;
    util::if_match(
        &headers,
        &format!("{ENDPOINT_KIND}-{endpoint_id}-v{}", doc.version),
    )?;
    let fields = endpoint_fields(map)?;
    let mut next = doc.body.clone();
    for key in ["name", "url", "tokenRef", "enabled"] {
        next[key] = fields[key].clone();
    }
    next["updatedAt"] = Value::String(now());
    state
        .store
        .update_document(ENDPOINT_KIND, &endpoint_id, &next, Some(doc.version))
        .await?;
    Ok(util::ok(&with_etag(
        next,
        ENDPOINT_KIND,
        &endpoint_id,
        doc.version + 1,
    )))
}

async fn delete_endpoint(
    State(state): State<Shared>,
    Path(endpoint_id): Path<String>,
) -> Result<Response, ApiError> {
    state
        .store
        .delete_document(ENDPOINT_KIND, &endpoint_id)
        .await?;
    Ok(util::ok(
        &json!({"id": endpoint_id, "deleted": true}),
    ))
}

fn parse_targets(value: Option<&Value>) -> Result<Vec<Value>, ApiError> {
    let items = value
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .ok_or_else(|| ApiError::validation("targets", "required", "至少选择一个投递目标"))?;
    let mut targets = Vec::new();
    for item in items {
        let kind = item["kind"].as_str().unwrap_or_default();
        let id = item["id"].as_str().unwrap_or_default().trim();
        if !matches!(kind, "model" | "endpoint" | "channel") || id.is_empty() {
            return Err(ApiError::validation(
                "targets",
                "format",
                "目标需要 kind（model/endpoint/channel）与非空 id",
            ));
        }
        targets.push(json!({"kind": kind, "id": id}));
    }
    Ok(targets)
}

/// 内容与任务里的 `sourceId` 带 `source/` 前缀，而信源目录与计划目标用 slug
/// 两种写法都带上，避免前缀差异导致规则匹配不到新内容
fn source_forms(id: &str) -> Vec<String> {
    let bare = id.trim().trim_start_matches("source/");
    if bare.is_empty() {
        return Vec::new();
    }
    vec![bare.to_string(), format!("source/{bare}")]
}

/// 监听对象：信源直接给出；计划换算成它包含的信源集合。
async fn listener_sources(state: &AppState, listener: &Value) -> Vec<String> {
    let kind = listener["kind"].as_str().unwrap_or_default();
    let id = listener["id"].as_str().unwrap_or_default();
    if kind == "source" {
        return source_forms(id);
    }
    if kind != "plan" || id.is_empty() {
        return Vec::new();
    }
    let entries = config::entries(&state.config_dir, "plan").unwrap_or_default();
    let Some(entry) = entries.into_iter().find(|entry| entry.slug == id) else {
        return Vec::new();
    };
    entry.doc["targets"]
        .as_array()
        .map(|targets| {
            targets
                .iter()
                .filter_map(Value::as_str)
                .map(|target| target.trim_start_matches("source/").to_string())
                .collect()
        })
        .unwrap_or_default()
}

async fn listener_view(state: &AppState, listener: &Value) -> Value {
    let (kind, id) = (
        listener["kind"].as_str().unwrap_or_default(),
        listener["id"].as_str().unwrap_or_default(),
    );
    let name = if kind == "source" {
        crate::catalog::source_rows(state)
            .await
            .ok()
            .and_then(|rows| {
                rows.into_iter()
                    .find(|row| row["id"].as_str() == Some(id))
                    .and_then(|row| row["name"].as_str().map(str::to_string))
            })
    } else {
        config::entries(&state.config_dir, "plan")
            .ok()
            .and_then(|entries| {
                entries
                    .into_iter()
                    .find(|entry| entry.slug == id)
                    .and_then(|entry| entry.doc["name"].as_str().map(str::to_string))
            })
    };
    json!({"kind": kind, "id": id, "name": name.unwrap_or_else(|| id.to_string())})
}

async fn rule_view(state: &AppState, body: &Value, version: i64, id: &str) -> Value {
    let mut view = body.clone();
    view["listener"] = listener_view(state, &body["listener"]).await;
    with_etag(view, RULE_KIND, id, version)
}

/// 监听对象归属：计划取计划所属项目，信源取信源归属；无法解析时按默认项目承接。
async fn listener_project(state: &AppState, listener: &Value) -> String {
    let kind = listener["kind"].as_str().unwrap_or("source");
    let id = listener["id"].as_str().unwrap_or_default();
    if kind == "plan" {
        if let Ok(plans) = crate::collection::plan_rows(state).await
            && let Some(project) = plans
                .iter()
                .find(|plan| plan["id"] == id)
                .and_then(|plan| plan["project"].as_str())
        {
            return project.to_string();
        }
        return crate::projects::DEFAULT_PROJECT_ID.to_string();
    }
    let slug = id.trim_start_matches("source/");
    crate::catalog::source_rows(state)
        .await
        .ok()
        .and_then(|rows| {
            rows.into_iter()
                .find(|row| row["id"].as_str() == Some(slug))
                .and_then(|row| row["project"].as_str().map(str::to_string))
        })
        .unwrap_or_else(|| crate::projects::DEFAULT_PROJECT_ID.to_string())
}

/// 规则归属：显式 project 优先（迁移/导入可直写），否则按监听对象派生。
async fn rule_project(state: &AppState, body: &Value) -> String {
    if let Some(project) = body["project"].as_str().filter(|value| !value.is_empty()) {
        return project.to_string();
    }
    listener_project(state, &body["listener"]).await
}

async fn list_rules(
    State(state): State<Shared>,
    project: ActiveProject,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let tag = "delivery_rule";
    let offset = cursor_offset(query.cursor.as_deref(), tag)?;
    let mut docs = Vec::new();
    for doc in state.store.list_documents(RULE_KIND).await? {
        if let Some(listener) = query.kind.clone().filter(|value| !value.is_empty())
            && doc.body["listener"]["kind"].as_str().unwrap_or_default() != listener
        {
            continue;
        }
        // 项目隔离：规则归属由监听对象派生，他项目的规则不进入列表。
        if rule_project(&state, &doc.body).await != project.id() {
            continue;
        }
        docs.push(rule_view(&state, &doc.body, doc.version, &doc.id).await);
    }
    docs.sort_by(|left, right| {
        right["createdAt"]
            .as_str()
            .unwrap_or("")
            .cmp(left["createdAt"].as_str().unwrap_or(""))
    });
    let total = docs.len();
    let items: Vec<Value> = docs.into_iter().skip(offset).take(limit + 1).collect();
    let more = items.len() > limit;
    let items: Vec<Value> = items.into_iter().take(limit).collect();
    let next = if more || offset + items.len() < total {
        Some(cursor(tag, offset + limit))
    } else {
        None
    };
    Ok(util::ok(&page(items, next)))
}

fn rule_fields(map: &Map<String, Value>) -> Result<Value, ApiError> {
    let name = util::non_empty(map, "name")?;
    let listener = map
        .get("listener")
        .ok_or_else(|| ApiError::validation("listener", "required", "需要选择监听的信源或计划"))?;
    let kind = listener["kind"].as_str().unwrap_or_default();
    let id = listener["id"].as_str().unwrap_or_default().trim();
    if !matches!(kind, "source" | "plan") || id.is_empty() {
        return Err(ApiError::validation(
            "listener",
            "format",
            "监听对象需要 kind（source/plan）与非空 id",
        ));
    }
    let targets = parse_targets(map.get("targets"))?;
    let enabled = util::optional_flag(map, "enabled", true)?;
    Ok(json!({
        "name": name,
        "listener": {"kind": kind, "id": id},
        "targets": targets,
        "enabled": enabled,
        "lastRunAt": Value::Null,
    }))
}

async fn create_rule(
    State(state): State<Shared>,
    project: ActiveProject,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["name", "listener", "targets", "enabled"])?;
    let fields = rule_fields(map)?;
    // 归属校验：只能监听当前项目的信源或计划。
    if listener_project(&state, &fields["listener"]).await != project.id() {
        return Err(ApiError::validation(
            "listener",
            "forbidden",
            "监听对象不属于当前项目",
        ));
    }
    let id = new_id("rule");
    let mut doc = fields;
    doc["id"] = Value::String(id.clone());
    doc["project"] = Value::String(project.id().to_string());
    doc["createdAt"] = Value::String(now());
    doc["updatedAt"] = Value::String(now());
    state.store.insert_document(RULE_KIND, &id, &doc).await?;
    Ok(util::created(&rule_view(&state, &doc, 1, &id).await))
}

async fn update_rule(
    State(state): State<Shared>,
    project: ActiveProject,
    Path(rule_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["name", "listener", "targets", "enabled"])?;
    let doc = state
        .store
        .get_document(RULE_KIND, &rule_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("自动投递规则 {rule_id}")))?;
    if rule_project(&state, &doc.body).await != project.id() {
        return Err(ApiError::not_found(format!("自动投递规则 {rule_id}")));
    }
    util::if_match(&headers, &format!("{RULE_KIND}-{rule_id}-v{}", doc.version))?;
    let mut next = doc.body.clone();
    // PATCH 语义：未提交的字段保持原值。
    if map.contains_key("name")
        || map.contains_key("listener")
        || map.contains_key("targets")
        || map.contains_key("enabled")
    {
        let merged = json!({
            "name": map.get("name").cloned().unwrap_or_else(|| doc.body["name"].clone()),
            "listener": map.get("listener").cloned().unwrap_or_else(|| doc.body["listener"].clone()),
            "targets": map.get("targets").cloned().unwrap_or_else(|| doc.body["targets"].clone()),
            "enabled": map.get("enabled").cloned().unwrap_or_else(|| doc.body["enabled"].clone()),
        });
        let fields = rule_fields(util::object(&merged)?)?;
        // 换监听对象也要留在当前项目内。
        if map.contains_key("listener")
            && listener_project(&state, &fields["listener"]).await != project.id()
        {
            return Err(ApiError::validation(
                "listener",
                "forbidden",
                "监听对象不属于当前项目",
            ));
        }
        for key in ["name", "listener", "targets", "enabled"] {
            next[key] = fields[key].clone();
        }
        // 监听对象与目标改变后重新开始计数，避免回补历史内容。
        if map.contains_key("listener") || map.contains_key("targets") {
            next["lastRunAt"] = Value::Null;
        }
    }
    next["updatedAt"] = Value::String(now());
    state
        .store
        .update_document(RULE_KIND, &rule_id, &next, Some(doc.version))
        .await?;
    Ok(util::ok(
        &rule_view(&state, &next, doc.version + 1, &rule_id).await,
    ))
}

async fn delete_rule(
    State(state): State<Shared>,
    project: ActiveProject,
    Path(rule_id): Path<String>,
) -> Result<Response, ApiError> {
    let doc = state
        .store
        .get_document(RULE_KIND, &rule_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("自动投递规则 {rule_id}")))?;
    if rule_project(&state, &doc.body).await != project.id() {
        return Err(ApiError::not_found(format!("自动投递规则 {rule_id}")));
    }
    state.store.delete_document(RULE_KIND, &rule_id).await?;
    Ok(util::ok(&json!({"id": rule_id, "deleted": true})))
}

/// 计划维度的投递目标：一个计划对应一条 `listener.kind = "plan"` 的规则。
///
/// 计划表单只提交目标与开关；规则名称、监听对象与增量计数都由这里维护，
/// 不把投递细节写进计划 YAML，避免配置层与运行态双写。
pub(crate) async fn sync_plan_rule(
    state: &AppState,
    plan_id: &str,
    plan_name: &str,
    targets: Option<&Value>,
    enabled: Option<bool>,
) -> Result<(), ApiError> {
    let existing = state
        .store
        .list_documents(RULE_KIND)
        .await?
        .into_iter()
        .find(|doc| {
            doc.body["listener"]["kind"].as_str() == Some("plan")
                && doc.body["listener"]["id"].as_str() == Some(plan_id)
        });
    let Some(targets) = targets else {
        // 未提交目标：至多改开关，且仅在规则已存在时。
        if let (Some(doc), Some(enabled)) = (existing, enabled) {
            let mut next = doc.body.clone();
            next["enabled"] = json!(enabled);
            next["updatedAt"] = Value::String(now());
            state
                .store
                .update_document(RULE_KIND, &doc.id, &next, Some(doc.version))
                .await?;
        }
        return Ok(());
    };
    if targets.as_array().map(Vec::is_empty).unwrap_or(true) {
        if let Some(doc) = existing {
            state.store.delete_document(RULE_KIND, &doc.id).await?;
        }
        return Ok(());
    }
    let next_targets = parse_targets(Some(targets))?;
    let enabled = enabled.unwrap_or_else(|| {
        existing
            .as_ref()
            .and_then(|doc| doc.body["enabled"].as_bool())
            .unwrap_or(true)
    });
    let name = if plan_name.trim().is_empty() {
        format!("计划 {plan_id} · 投递")
    } else {
        format!("{} · 投递", plan_name.trim())
    };
    match existing {
        Some(doc) => {
            // 目标变了就重新开始计数，避免回补历史内容。
            let changed = doc.body["targets"] != Value::Array(next_targets.clone());
            let mut next = doc.body.clone();
            next["name"] = Value::String(name);
            next["targets"] = Value::Array(next_targets);
            next["enabled"] = json!(enabled);
            if changed {
                next["lastRunAt"] = Value::Null;
            }
            next["updatedAt"] = Value::String(now());
            state
                .store
                .update_document(RULE_KIND, &doc.id, &next, Some(doc.version))
                .await?;
        }
        None => {
            let id = new_id("rule");
            // 计划维度的规则随计划归属写入 project，列表与鉴权无需再逐条派生。
            let project = listener_project(state, &json!({"kind": "plan", "id": plan_id})).await;
            let doc = json!({
                "id": id,
                "name": name,
                "project": project,
                "listener": {"kind": "plan", "id": plan_id},
                "targets": next_targets,
                "enabled": enabled,
                "lastRunAt": Value::Null,
                "createdAt": now(),
                "updatedAt": now(),
            });
            state.store.insert_document(RULE_KIND, &id, &doc).await?;
        }
    }
    Ok(())
}

/// 计划 → 投递目标视图，供计划列表与表单回填。
pub(crate) async fn plan_rule_views(
    state: &AppState,
) -> Result<std::collections::HashMap<String, Value>, ApiError> {
    let mut map = std::collections::HashMap::new();
    for doc in state.store.list_documents(RULE_KIND).await? {
        if doc.body["listener"]["kind"].as_str() != Some("plan") {
            continue;
        }
        let Some(plan_id) = doc.body["listener"]["id"].as_str() else {
            continue;
        };
        map.insert(
            plan_id.to_string(),
            json!({
                "ruleId": doc.id,
                "targets": doc.body["targets"].clone(),
                "enabled": doc.body["enabled"].clone(),
                "lastRunAt": doc.body["lastRunAt"].clone(),
                "lastStatus": doc.body["lastStatus"].clone(),
            }),
        );
    }
    Ok(map)
}

/// 计划删除时一并清掉它的投递规则。
pub(crate) async fn remove_plan_rule(state: &AppState, plan_id: &str) -> Result<(), ApiError> {
    for doc in state.store.list_documents(RULE_KIND).await? {
        if doc.body["listener"]["kind"].as_str() == Some("plan")
            && doc.body["listener"]["id"].as_str() == Some(plan_id)
        {
            state.store.delete_document(RULE_KIND, &doc.id).await?;
        }
    }
    Ok(())
}

async fn list_records(
    State(state): State<Shared>,
    project: ActiveProject,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let kind = query.kind.clone();
    let status = query.status.clone();
    let active = project.id().to_string();
    page_docs(&state, RECORD_KIND, query, move |body| {
        // 项目隔离：旧记录没有 project，按默认项目承接，不向其他项目泄露。
        let owner = body["project"]
            .as_str()
            .filter(|value| !value.is_empty())
            .unwrap_or(crate::projects::DEFAULT_PROJECT_ID);
        if owner != active {
            return false;
        }
        if let Some(kind) = kind.as_deref().filter(|value| !value.is_empty())
            && body["type"].as_str() != Some(kind)
        {
            return false;
        }
        if let Some(status) = status.as_deref().filter(|value| !value.is_empty())
            && body["status"].as_str() != Some(status)
        {
            return false;
        }
        true
    })
    .await
}

async fn get_record(
    State(state): State<Shared>,
    project: ActiveProject,
    Path(record_id): Path<String>,
) -> Result<Response, ApiError> {
    let doc = state
        .store
        .get_document(RECORD_KIND, &record_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("投递记录 {record_id}")))?;
    let owner = doc.body["project"]
        .as_str()
        .filter(|value| !value.is_empty())
        .unwrap_or(crate::projects::DEFAULT_PROJECT_ID);
    if owner != project.id() {
        return Err(ApiError::not_found(format!("投递记录 {record_id}")));
    }
    Ok(util::ok(&with_etag(
        doc.body,
        RECORD_KIND,
        &record_id,
        doc.version,
    )))
}

/// 手动投递：先落记录与 Operation，再由后台逐个目标提交。
async fn create_job(
    State(state): State<Shared>,
    project: ActiveProject,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["contentIds", "targets", "note"])?;
    let content_ids = util::strings(map, "contentIds")?;
    if content_ids.is_empty() {
        return Err(ApiError::validation("contentIds", "required", "至少选择一条内容"));
    }
    let targets = parse_targets(map.get("targets"))?;
    let note = util::optional_text(map, "note")?.unwrap_or_default();
    let mut items = Vec::new();
    for id in &content_ids {
        items.push(crate::content::delivery_snapshot(&state, id, project.id()).await?);
    }
    let record_id = new_id("send");
    let record = json!({
        "id": record_id,
        "type": "manual",
        "status": "queued",
        "project": project.id(),
        "note": note,
        "trigger": {"kind": "manual", "by": "控制台"},
        "items": items.iter().map(|item| json!({
            "contentId": item["contentId"],
            "title": item["title"],
            "sourceName": item["sourceName"],
        })).collect::<Vec<_>>(),
        "targets": targets,
        "results": [],
        "createdAt": now(),
        "updatedAt": now(),
    });
    state
        .store
        .insert_document(RECORD_KIND, &record_id, &record)
        .await?;
    let operation = crate::operations::start(
        &state,
        "delivery.manual",
        vec![json!({"kind": "delivery", "id": record_id})],
        json!({
            "kind": "delivery.manual",
            "recordId": record_id,
            "contentIds": content_ids,
            "targets": targets,
            "projectId": project.id(),
        }),
    )
    .await?;
    Ok(accepted(&operation))
}

pub async fn execute_manual(
    state: &AppState,
    operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let record_id = action["recordId"].as_str().unwrap_or_default();
    let project_id = action["projectId"]
        .as_str()
        .filter(|value| !value.is_empty())
        .unwrap_or(crate::projects::DEFAULT_PROJECT_ID);
    let items = action["items"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    // items 未随 action 持久化时按内容标识重新取快照（恢复执行也走这里）。
    let mut snapshots = Vec::new();
    if items.is_empty() {
        for id in action["contentIds"].as_array().cloned().unwrap_or_default() {
            if let Some(id) = id.as_str() {
                snapshots.push(crate::content::delivery_snapshot(state, id, project_id).await?);
            }
        }
    }
    let doc = state
        .store
        .get_document(RECORD_KIND, record_id)
        .await?
        .ok_or_else(|| ApiError::not_found(record_id))?;
    let mut record = doc.body.clone();
    record["status"] = Value::String("running".into());
    record["updatedAt"] = Value::String(now());
    state
        .store
        .update_document(RECORD_KIND, record_id, &record, Some(doc.version))
        .await?;
    let outcome = run_record(state, operation_id, &mut record, &snapshots).await?;
    let current = state
        .store
        .get_document(RECORD_KIND, record_id)
        .await?
        .ok_or_else(|| ApiError::not_found(record_id))?;
    state
        .store
        .update_document(RECORD_KIND, record_id, &record, Some(current.version))
        .await?;
    Ok(outcome)
}

/// 逐目标投递并写回 record.results；返回 Operation 结果摘要。
async fn run_record(
    state: &AppState,
    operation_id: &str,
    record: &mut Value,
    snapshots: &[Value],
) -> Result<Value, ApiError> {
    let targets = record["targets"].as_array().cloned().unwrap_or_default();
    let items = if snapshots.is_empty() {
        record["items"].as_array().cloned().unwrap_or_default()
    } else {
        snapshots.to_vec()
    };
    let mut succeeded = 0usize;
    let mut failed = 0usize;
    let mut results = Vec::new();
    for target in &targets {
        let kind = target["kind"].as_str().unwrap_or_default();
        let id = target["id"].as_str().unwrap_or_default();
        if crate::operations::check_cancel(state, operation_id).await.is_err() {
            record["status"] = Value::String("cancelled".into());
            record["updatedAt"] = Value::String(now());
            return Ok(json!({"partial": false, "cancelled": true, "recordId": record["id"]}));
        }
        let started = std::time::Instant::now();
        match deliver_target(state, kind, id, &items).await {
            Ok(output) => {
                succeeded += 1;
                results.push(json!({
                    "targetKind": kind,
                    "targetId": id,
                    "targetName": target["name"],
                    "status": "succeeded",
                    "latencyMs": started.elapsed().as_millis() as u64,
                    "output": output,
                }));
            }
            Err(error) => {
                failed += 1;
                results.push(json!({
                    "targetKind": kind,
                    "targetId": id,
                    "targetName": target["name"],
                    "status": "failed",
                    "latencyMs": started.elapsed().as_millis() as u64,
                    "error": error.message.clone(),
                }));
            }
        }
    }
    record["results"] = Value::Array(results.clone());
    record["status"] = Value::String(if failed == 0 {
        "succeeded".into()
    } else if succeeded == 0 {
        "failed".into()
    } else {
        "partiallySucceeded".into()
    });
    record["updatedAt"] = Value::String(now());
    Ok(json!({
        "partial": failed > 0 && succeeded > 0,
        "recordId": record["id"],
        "status": record["status"],
        "succeeded": succeeded,
        "failed": failed,
    }))
}

async fn deliver_target(
    state: &AppState,
    kind: &str,
    id: &str,
    items: &[Value],
) -> Result<Value, ApiError> {
    match kind {
        "model" => {
            let doc = state
                .store
                .get_document("model", id)
                .await?
                .ok_or_else(|| ApiError::not_found(format!("模型 {id}")))?;
            if !doc.body["enabled"].as_bool().unwrap_or(false) {
                return Err(ApiError::conflict(format!("模型 {id} 已停用")));
            }
            let spec = crate::models::model_spec(&doc.body)?;
            let prompt = model_prompt(items);
            let text = workflow::complete_model(&spec, &prompt, &state.services.proxy_setting())
                .await
                .map_err(|error| ApiError::dependency(error.to_string()))?;
            Ok(json!({"kind": "model", "text": text}))
        }
        "endpoint" => {
            let doc = state
                .store
                .get_document(ENDPOINT_KIND, id)
                .await?
                .ok_or_else(|| ApiError::not_found(format!("外部 API {id}")))?;
            if !doc.body["enabled"].as_bool().unwrap_or(true) {
                return Err(ApiError::conflict(format!("外部 API {id} 已停用")));
            }
            let url = doc.body["url"].as_str().unwrap_or_default();
            let mut headers = vec![("content-type".to_string(), "application/json".to_string())];
            if let Some(name) = doc.body["tokenRef"]
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                && let Ok(token) = std::env::var(name)
            {
                headers.push(("authorization".to_string(), format!("Bearer {token}")));
            }
            let payload = json!({
                "source": "ModalPrism",
                "kind": "contentDelivery",
                "title": record_title(items),
                "items": items,
            });
            let reply = workflow::http_request(
                "POST",
                url,
                &headers,
                Some(payload.to_string()),
                Duration::from_secs(20),
                &state.services.proxy_setting(),
            )
            .await
            .map_err(|error| ApiError::dependency(error.to_string()))?;
            let text = reply
                .ok("外部 API")
                .map_err(|error| ApiError::dependency(error.to_string()))?;
            Ok(json!({"kind": "endpoint", "status": 200, "sample": text.chars().take(400).collect::<String>()}))
        }
        "channel" => {
            let outcome = crate::channels::deliver_to_channel(
                state,
                id,
                &record_title(items),
                &record_text(items),
                None,
            )
            .await?;
            Ok(json!({"kind": "channel", "detail": outcome}))
        }
        other => Err(ApiError::validation(
            "targets",
            "format",
            format!("未知投递目标类型: {other}"),
        )),
    }
}

fn record_title(items: &[Value]) -> String {
    if items.len() == 1 {
        items[0]["title"]
            .as_str()
            .unwrap_or("内容投递")
            .to_string()
    } else {
        format!("内容投递 · {} 条", items.len())
    }
}

fn record_text(items: &[Value]) -> String {
    items
        .iter()
        .map(|item| {
            let title = item["title"].as_str().unwrap_or("未命名");
            let source = item["sourceName"].as_str().unwrap_or("");
            let summary = item["summary"].as_str().unwrap_or("");
            format!("· {title}（{source}）\n  {summary}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn model_prompt(items: &[Value]) -> String {
    let mut prompt = String::from(
        "下面是从内容库投递过来的条目。请用简洁中文逐条给出标题、要点与可执行结论，不要编造原文没有的信息。\n\n",
    );
    for (index, item) in items.iter().enumerate() {
        let text = item["text"]
            .as_str()
            .filter(|value| !value.is_empty())
            .unwrap_or(item["summary"].as_str().unwrap_or(""));
        prompt.push_str(&format!(
            "[{}] 标题：{}\n来源：{}\n正文：{}\n\n",
            index + 1,
            item["title"].as_str().unwrap_or("未命名"),
            item["sourceName"].as_str().unwrap_or(""),
            text.chars().take(4000).collect::<String>(),
        ));
    }
    prompt
}

/// 自动投递轮询：按规则增量取新内容并投递。订阅式实现，不改采集链路。
pub async fn auto_round(state: &AppState) -> Result<usize, ApiError> {
    let rules = state.store.list_documents(RULE_KIND).await?;
    let mut sent = 0usize;
    for doc in rules {
        if !doc.body["enabled"].as_bool().unwrap_or(false) {
            continue;
        }
        let sources = listener_sources(state, &doc.body["listener"]).await;
        if sources.is_empty() {
            continue;
        }
        let since = doc.body["lastRunAt"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| doc.body["createdAt"].as_str().unwrap_or("").to_string());
        let rids = new_content_ids(state, &sources, &since).await?;
        if rids.is_empty() {
            continue;
        }
        let mut items = Vec::new();
        // 自动投递按规则所属项目取快照，不再统一落到默认项目。
        let owner = rule_project(state, &doc.body).await;
        for rid in &rids {
            if let Ok(snapshot) = crate::content::delivery_snapshot(state, rid, &owner).await {
                items.push(snapshot);
            }
        }
        if items.is_empty() {
            continue;
        }
        let record_id = new_id("auto");
        let mut record = json!({
            "id": record_id,
            "type": "auto",
            "status": "running",
            "project": owner,
            "ruleId": doc.id,
            "trigger": {
                "kind": "rule",
                "ruleName": doc.body["name"],
                "listener": listener_view(state, &doc.body["listener"]).await,
            },
            "items": items.iter().map(|item| json!({
                "contentId": item["contentId"],
                "title": item["title"],
                "sourceName": item["sourceName"],
            })).collect::<Vec<_>>(),
            "targets": doc.body["targets"],
            "results": [],
            "createdAt": now(),
            "updatedAt": now(),
        });
        let outcome = run_record(state, "auto", &mut record, &items).await?;
        state
            .store
            .insert_document(RECORD_KIND, &record_id, &record)
            .await?;
        let mut next = doc.body.clone();
        next["lastRunAt"] = Value::String(now());
        next["lastRecordId"] = Value::String(record_id);
        next["lastStatus"] = outcome["status"].clone();
        next["updatedAt"] = Value::String(now());
        state
            .store
            .update_document(RULE_KIND, &doc.id, &next, Some(doc.version))
            .await?;
        let status = outcome["status"].as_str().unwrap_or("succeeded");
        let bad = status != "succeeded";
        let rule_name = doc.body["name"].as_str().unwrap_or("未命名规则");
        crate::notifications::emit(
            state,
            "delivery",
            if bad {
                "delivery.failed"
            } else {
                "content.delivered"
            },
            if bad { "warn" } else { "info" },
            format!("自动投递{}：{rule_name}", if bad { "未完成" } else { "完成" }),
            format!(
                "{} 条内容 · 成功 {} / 失败 {}",
                items.len(),
                outcome["succeeded"].as_u64().unwrap_or(0),
                outcome["failed"].as_u64().unwrap_or(0)
            ),
            Some("delivery?tab=records"),
        )
        .await;
        sent += 1;
    }
    Ok(sent)
}

async fn new_content_ids(
    state: &AppState,
    sources: &[String],
    since: &str,
) -> Result<Vec<String>, ApiError> {
    let filters: Vec<Value> = vec![
        json!({"in": ["sourceId", sources]}),
        json!({"gt": ["createdAt", since]}),
    ];
    let spec = StrataQuery {
        from: QueryFrom::Resources,
        r#where: serde_json::from_value::<Expr>(json!({"all": filters})).ok(),
        has_facets: Vec::new(),
        select: vec![
            SelectItem::Field("rid".into()),
            SelectItem::Field("createdAt".into()),
        ],
        expand: Vec::new(),
        search: None,
        as_of: None,
        order_by: vec![OrderBy {
            field: "createdAt".into(),
            descending: false,
        }],
        limit: Some(AUTO_BATCH as u32),
        cursor: None,
        include_tombstoned: false,
    };
    let page = state
        .services
        .strata()
        .query(spec)
        .await
        .map_err(ApiError::from)?;
    Ok(page
        .rows
        .iter()
        .filter_map(|row| row["rid"].as_str().map(str::to_string))
        .collect())
}

/// 进程内启动自动投递轮询；重复调用是空操作。
pub async fn spawn_auto_sweeper(state: Shared) {
    let _ = SWEEPER
        .get_or_init(|| async {
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_secs(60));
                loop {
                    tick.tick().await;
                    if state.in_maintenance() {
                        continue;
                    }
                    if SWEEPER_BUSY
                        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                        .is_err()
                    {
                        continue;
                    }
                    let outcome = auto_round(&state).await;
                    SWEEPER_BUSY.store(false, Ordering::SeqCst);
                    if let Ok(count) = outcome
                        && count > 0
                    {
                        crate::observe::append_audit(
                            &state,
                            "delivery.auto.round",
                            "succeeded",
                            "自动投递轮询",
                            json!({"records": count}),
                            None,
                        )
                        .await
                        .ok();
                    }
                }
            });
        })
        .await;
}
