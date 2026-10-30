//! 信源、访问方式、适配器与 AI 生成物门禁。

use std::path::Path;

use axum::Json;
use axum::Router;
use axum::extract::{Path as AxPath, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::error::ApiError;
use crate::observe::Refs;
use crate::operations::accepted;
use crate::util::{self, cursor, page_total};
use crate::{AppState, Shared, new_id, now};

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/catalog/sources", get(list_sources).post(create_source))
        .route(
            "/catalog/sources/{sourceId}",
            get(get_source).patch(update_source).delete(delete_source),
        )
        .route("/catalog/sources/{sourceId}/enable", post(enable_source))
        .route("/catalog/sources/{sourceId}/disable", post(disable_source))
        .route("/catalog/sources/{sourceId}/share", post(share_source))
        .route("/catalog/sources/{sourceId}/unshare", post(unshare_source))
        .route("/catalog/sources/{sourceId}/probe", post(probe_source))
        .route("/catalog/accessMethods", get(list_access_methods))
        .route("/catalog/sourceTypes", get(list_source_types))
        .route("/catalog/adapters", get(list_adapters))
        .route("/catalog/templates", get(list_templates))
        .route("/catalog/sourceAnalyses", post(create_source_analysis))
        .route(
            "/catalog/adapterDrafts",
            get(list_adapter_drafts).post(create_adapter_draft),
        )
        .route(
            "/catalog/adapterDrafts/{draftId}/approve",
            post(approve_adapter_draft),
        )
        .route(
            "/catalog/adapterDrafts/{draftId}/reject",
            post(reject_adapter_draft),
        )
        // 信源分类与目录同源：一并挂到 catalog 路由，避免两处维护端点清单。
        .merge(crate::source_groups::routes())
}

#[derive(Debug, Deserialize)]
pub(crate) struct ListQuery {
    #[serde(default)]
    pub(crate) limit: Option<String>,
    #[serde(default)]
    pub(crate) cursor: Option<String>,
    #[serde(default)]
    pub(crate) offset: Option<String>,
    #[serde(default)]
    pub(crate) query: Option<String>,
    #[serde(default)]
    pub(crate) status: Option<String>,
    /// `all` 取消项目隔离过滤（管理与迁移用）；缺省按活跃项目过滤。
    #[serde(default)]
    pub(crate) scope: Option<String>,
}

pub(crate) fn options() -> config::LoadOptions {
    config::LoadOptions::from_process()
}

/// 递归判断声明正文是否含 `binding/...` 引用（H8 过渡守卫用）。
fn doc_has_binding_refs(value: &Value) -> bool {
    match value {
        Value::String(text) => text.starts_with("binding/"),
        Value::Array(items) => items.iter().any(doc_has_binding_refs),
        Value::Object(map) => map.values().any(doc_has_binding_refs),
        _ => false,
    }
}

pub(crate) async fn source_rows(state: &AppState) -> Result<Vec<Value>, ApiError> {
    let list = config::list(&state.config_dir)?;
    let statuses = state
        .services
        .planner_statuses()
        .await
        .map_err(ApiError::from)?;
    let mut rows = Vec::new();
    // 采集节律已迁出信源、落到计划：按 target 建立 source→trigger 映射，回显节奏。
    let mut plan_by_target: std::collections::BTreeMap<String, Value> =
        std::collections::BTreeMap::new();
    for plan in list["plans"].as_array().cloned().unwrap_or_default() {
        let Some(trigger) = plan.get("trigger") else {
            continue;
        };
        for target in plan["targets"].as_array().cloned().unwrap_or_default() {
            let Some(id) = target.as_str() else {
                continue;
            };
            plan_by_target
                .entry(id.trim_start_matches("source/").to_string())
                .or_insert_with(|| trigger.clone());
        }
    }
    for source in list["sources"].as_array().cloned().unwrap_or_default() {
        let slug = source["slug"].as_str().unwrap_or_default();
        let doc = source.get("doc").cloned().unwrap_or_else(|| json!({}));
        let status = statuses
            .iter()
            .find(|(id, _)| id == slug || id == &format!("source/{slug}"))
            .map(|(_, status)| {
                json!({
                    "healthStatus": normalize_health(&status.health_status),
                    "nextAt": status.next_at,
                    "lastOkAt": status.last_ok_at,
                    "lastError": status.last_error,
                    "todayYield": status.today_yield,
                    "alert": status.alert,
                    "skipReason": status.skip_reason,
                })
            })
            .unwrap_or_else(|| {
                json!({
                    "healthStatus": if source["enabled"].as_bool().unwrap_or(true) { "healthy" } else { "paused" },
                    "nextAt": Value::Null,
                    "lastOkAt": Value::Null,
                    "lastError": Value::Null,
                    "todayYield": 0,
                    "alert": false,
                    "skipReason": Value::Null,
                })
            });
        let accessor = doc
            .pointer("/access/accessor")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim_start_matches("accessor/")
            .to_string();
        let adapters: Vec<String> = source["adapters"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|item| {
                item.as_str()
                    .map(|value| value.trim_start_matches("adapter/").to_string())
            })
            .collect();
        let trigger = plan_by_target.get(slug).cloned().unwrap_or(Value::Null);
        let interval = trigger
            .get("interval")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let mode = if accessor == "push" {
            "push"
        } else if trigger.get("adaptive").is_some() {
            "adaptive"
        } else if trigger.is_null() {
            "none"
        } else {
            "fixed"
        };
        let endpoint = doc
            .pointer("/access/locator/url")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let channel = doc
            .pointer("/access/locator/channel")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let etag = util::etag(&[slug, &util::hash_short(&doc.to_string())]);
        rows.push(json!({
            "id": slug,
            "name": source["name"],
            "description": source["description"],
            "enabled": source["enabled"],
            "project": doc.get("project").cloned()
                .unwrap_or_else(|| json!(crate::projects::DEFAULT_PROJECT_ID)),
            "shared": doc.get("shared").and_then(Value::as_bool).unwrap_or(false),
            "accessMethodId": accessor,
            "sourceType": config::source_type::effective(&doc).as_str(),
            "adapterIds": adapters,
            "endpoint": endpoint,
            "channel": channel,
            "schedule": {
                "mode": mode,
                "intervalMs": util::duration_ms(interval),
                "interval": interval,
            },
            "health": status.get("healthStatus").cloned().unwrap_or(Value::Null),
            "healthReason": status.get("lastError").cloned().unwrap_or(Value::Null),
            "nextRunAt": status.get("nextAt").cloned().unwrap_or(Value::Null),
            "lastRunAt": status.get("lastOkAt").cloned().unwrap_or(Value::Null),
            "todayYield": status.get("todayYield").cloned().unwrap_or(Value::Null),
            "alert": status.get("alert").cloned().unwrap_or(Value::Null),
            "stopReason": status.get("skipReason").cloned().unwrap_or(Value::Null),
            "references": source["references"],
            "labels": doc.get("labels").cloned().unwrap_or_else(|| json!({})),
            // 用户自定义分类；分类元数据单独存放，这里只回显成员关系。
            "groupId": doc.get("groupId").cloned().unwrap_or(Value::Null),
            "etag": etag,
            "updatedAt": Value::Null,
        }));
    }
    Ok(rows)
}

fn normalize_health(raw: &str) -> &str {
    match raw {
        "ok" | "healthy" => "healthy",
        "misconfigured" | "configurationError" => "configurationError",
        "paused" => "paused",
        "degraded" => "degraded",
        "failed" => "failed",
        "suspicious" => "suspicious",
        other => other,
    }
}

/// 信源对某项目是否可见：归属该项目，或被标记共享。
pub(crate) fn source_visible(row: &Value, project_id: &str) -> bool {
    row["shared"].as_bool().unwrap_or(false) || row["project"].as_str() == Some(project_id)
}

/// 信源是否可被某项目修改：仅归属该项目时（共享只放开读，不放开写）。
pub(crate) fn source_editable(row: &Value, project_id: &str) -> bool {
    row["project"].as_str() == Some(project_id)
}

/// 某项目**自有**（非共享）的信源 id 集合，同样是 slug 与 `source/{slug}` 两种写法。
///
/// 项目是数据归属的根：信源归属项目，内容、任务、运行由信源派生归属；
/// 共享只放开“信源声明”的跨项目只读，不能让共享信源产出的内容/任务跑到其他项目。
/// 因此内容与运行类列表一律用自有集合过滤，信源列表才用可见集合。
pub(crate) async fn owned_source_ids(
    state: &AppState,
    project_id: &str,
) -> Result<std::collections::HashSet<String>, ApiError> {
    let rows = source_rows(state).await?;
    let mut ids = std::collections::HashSet::new();
    for row in rows
        .iter()
        .filter(|row| crate::catalog::source_editable(row, project_id))
    {
        if let Some(slug) = row["id"].as_str() {
            ids.insert(slug.to_string());
            ids.insert(format!("source/{slug}"));
        }
    }
    Ok(ids)
}

/// 某类型的全部信源 id（slug 与 `source/{slug}` 两种写法），供内容按类型过滤。
pub(crate) async fn source_ids_of_type(
    state: &AppState,
    kind: &str,
) -> Result<std::collections::HashSet<String>, ApiError> {
    let rows = source_rows(state).await?;
    let mut ids = std::collections::HashSet::new();
    for row in rows
        .iter()
        .filter(|row| row["sourceType"].as_str() == Some(kind))
    {
        if let Some(slug) = row["id"].as_str() {
            ids.insert(slug.to_string());
            ids.insert(format!("source/{slug}"));
        }
    }
    Ok(ids)
}

async fn list_sources(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let filter = format!(
        "sources:{}:{}:{}",
        query.query.clone().unwrap_or_default(),
        query.status.clone().unwrap_or_default(),
        if query.scope.as_deref() == Some("all") {
            "all".to_string()
        } else {
            project.id().to_string()
        }
    );
    let offset = util::page_offset(query.offset.as_deref(), query.cursor.as_deref(), &filter)?;
    let needle = query
        .query
        .as_deref()
        .map(str::to_lowercase)
        .filter(|value| !value.is_empty());
    let status = query.status.as_deref().filter(|value| !value.is_empty());
    let mut rows = source_rows(&state).await?;
    // 项目隔离：仅显示归属当前项目或共享的信源；scope=all 取消过滤。
    if query.scope.as_deref() != Some("all") {
        let active = project.id();
        rows.retain(|row| source_visible(row, active));
    }
    if let Some(needle) = needle {
        rows.retain(|row| {
            row["name"]
                .as_str()
                .unwrap_or("")
                .to_lowercase()
                .contains(&needle)
                || row["id"]
                    .as_str()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&needle)
                || row["endpoint"]
                    .as_str()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&needle)
        });
    }
    if let Some(status) = status {
        rows.retain(|row| row["health"] == status);
    }
    let total = rows.len();
    let items: Vec<Value> = rows.into_iter().skip(offset).take(limit + 1).collect();
    let more = items.len() > limit;
    let items: Vec<Value> = items.into_iter().take(limit).collect();
    let next = if more || offset + items.len() < total {
        Some(cursor(&filter, offset + limit))
    } else {
        None
    };
    Ok(util::ok(&page_total(items, next, total)))
}

async fn get_source(
    State(state): State<Shared>,
    AxPath(source_id): AxPath<String>,
) -> Result<Response, ApiError> {
    let rows = source_rows(&state).await?;
    let row = rows
        .into_iter()
        .find(|row| row["id"] == source_id)
        .ok_or_else(|| ApiError::not_found(format!("信源 {source_id}")))?;
    let mut response = util::ok(&row);
    if let Ok(value) = row["etag"].as_str().unwrap_or("").parse() {
        response
            .headers_mut()
            .insert(axum::http::header::ETAG, value);
    }
    Ok(response)
}

fn source_doc(
    existing: Option<&Value>,
    map: &Map<String, Value>,
) -> Result<(Value, String), ApiError> {
    let name = util::non_empty(map, "name")?;
    let accessor = util::non_empty(map, "accessMethodId")?;
    let adapters = util::strings(map, "adapterIds")?;
    let enabled = util::flag(map, "enabled")?;
    let endpoint = util::optional_text(map, "endpoint")?;
    let channel = util::optional_text(map, "channel")?;
    let schedule = util::optional_object(map, "schedule")?;
    let admission = util::optional_object(map, "admission")?;
    let declared_type = util::optional_text(map, "sourceType")?;
    let accessor = if accessor.starts_with("accessor/") {
        accessor
    } else {
        format!("accessor/{accessor}")
    };
    let mut doc = existing
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    doc.insert("name".into(), Value::String(name.clone()));
    let mut access = doc
        .get("access")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    access.insert("accessor".into(), Value::String(accessor.clone()));
    let mut locator = access
        .get("locator")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let channel_value = channel
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string);
    if let Some(channel) = &channel_value {
        // 接收类只认频道标识：派生规则按 channel 优先，同一声明不能既是 url 源又是 push 源。
        locator.insert("channel".into(), Value::String(channel.clone()));
        locator.remove("url");
        locator.remove("endpoint");
    } else {
        let endpoint_value = endpoint.or_else(|| {
            locator
                .get("url")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
        if let Some(endpoint) = &endpoint_value {
            let parsed = url::Url::parse(endpoint)
                .map_err(|_| ApiError::validation("endpoint", "format", "不是有效地址"))?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return Err(ApiError::validation(
                    "endpoint",
                    "scheme",
                    "只接受 http/https",
                ));
            }
            locator.insert("url".into(), Value::String(endpoint.clone()));
        }
    }
    access.insert("locator".into(), Value::Object(locator));
    doc.insert("access".into(), Value::Object(access));
    if adapters.is_empty() {
        return Err(ApiError::validation(
            "adapterIds",
            "required",
            "至少选择一个适配器",
        ));
    }
    doc.insert(
        "pipeline".into(),
        json!([{
            "run": adapters.into_iter().map(|item| if item.starts_with("adapter/") { item } else { format!("adapter/{item}") }).collect::<Vec<_>>()
        }]),
    );
    doc.insert("enabled".into(), Value::Bool(enabled));
    // R12/R14/S3：类型闭集 + 一致性 + 矩阵校验。调用方可声明；缺省按装配后的形状推导，
    // 保证经 HTTP/MCP/Skill 写盘的信源始终带显式 sourceType，且与 accessor/adapter 组合自洽。
    let resolved = match declared_type.as_deref() {
        Some(text) => config::source_type::SourceType::parse(text).ok_or_else(|| {
            ApiError::validation(
                "sourceType",
                "closedSet",
                format!(
                    "未知类型: {text}（应为 {}）",
                    config::source_type::SourceType::closed_set_text()
                ),
            )
        })?,
        None => config::source_type::derive(&Value::Object(doc.clone())),
    };
    doc.insert("sourceType".into(), json!(resolved.as_str()));
    if resolved == config::source_type::SourceType::Push {
        let has_channel = doc
            .get("access")
            .and_then(|access| access.get("locator"))
            .and_then(|locator| locator.get("channel"))
            .and_then(Value::as_str)
            .is_some_and(|text| !text.is_empty());
        if !has_channel {
            return Err(ApiError::validation(
                "channel",
                "required",
                "接收类信源需要频道标识",
            ));
        }
        // 推送是事件驱动：计划退化为入口 + 准入，不保留轮询间隔。
        doc.remove("schedule");
    }
    if let Some(issue) = config::source_type::validate(&Value::Object(doc.clone()))
        .into_iter()
        .next()
    {
        return Err(ApiError::validation("sourceType", "combination", issue));
    }
    if let Some(schedule) = schedule {
        util::unknown(&schedule, &["mode", "intervalMs", "minimumIntervalMs"])?;
        let mode = util::text(&schedule, "mode")?;
        if mode == "push" || resolved == config::source_type::SourceType::Push {
            doc.remove("schedule");
        } else {
            let interval_ms = schedule
                .get("intervalMs")
                .and_then(Value::as_i64)
                .ok_or_else(|| {
                    ApiError::validation(
                        "schedule.intervalMs",
                        "required",
                        "固定或自适应模式需要间隔",
                    )
                })?;
            if interval_ms < 1000 {
                return Err(ApiError::validation(
                    "schedule.intervalMs",
                    "outOfRange",
                    "间隔过短",
                ));
            }
            doc.insert(
                "schedule".into(),
                json!({"interval": util::duration_text(interval_ms)}),
            );
        }
    }
    if let Some(admission) = admission {
        doc.insert("admission".into(), Value::Object(admission));
    }
    Ok((Value::Object(doc), name))
}

async fn create_source(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(
        map,
        &[
            "name",
            "accessMethodId",
            "adapterIds",
            "enabled",
            "endpoint",
            "channel",
            "schedule",
            "admission",
            "sourceType",
            "shared",
            "groupId",
        ],
    )?;
    let shared = util::optional_flag(map, "shared", false)?;
    let (mut doc, name) = source_doc(None, map)?;
    // 创建即归属活跃项目；共享只放开跨项目读。
    doc["project"] = json!(project.id());
    doc["shared"] = json!(shared);
    if let Some(group_id) = group_id_of(map)? {
        crate::source_groups::ensure_group(&state, &group_id, project.id()).await?;
        doc["groupId"] = json!(group_id);
    }
    let base = util::slug(&name);
    let mut slug = base.clone();
    for index in 2..20 {
        if !Path::new(&state.config_dir)
            .join("sources")
            .join(format!("{slug}.yaml"))
            .exists()
        {
            break;
        }
        slug = format!("{base}-{index}");
    }
    let saved = config::create(
        &state.config_dir,
        &options(),
        &config::Draft {
            kind: "source".into(),
            slug: slug.clone(),
            doc,
        },
    )?;
    state.services.config_reload()?;
    crate::observe::append_audit(
        &state,
        "catalog.source.create",
        "succeeded",
        "新建信源",
        json!({"id": saved.id, "name": name}),
        None,
    )
    .await?;
    let row = source_by_slug(&state, &slug).await?;
    Ok(util::created(&row))
}

async fn update_source(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    AxPath(source_id): AxPath<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(
        map,
        &[
            "name",
            "accessMethodId",
            "adapterIds",
            "enabled",
            "endpoint",
            "channel",
            "schedule",
            "admission",
            "sourceType",
            "shared",
            "groupId",
        ],
    )?;
    let current = source_by_slug(&state, &source_id).await?;
    if !source_editable(&current, project.id()) {
        return Err(ApiError::not_found(format!("信源 {source_id}")));
    }
    util::if_match(&headers, current["etag"].as_str().unwrap_or(""))?;
    let existing = config::entries(&state.config_dir, "source")?
        .into_iter()
        .find(|entry| entry.slug == source_id)
        .map(|entry| entry.doc)
        .ok_or_else(|| ApiError::not_found(format!("信源 {source_id}")))?;
    // H8 过渡守卫：本信源已含 binding 引用。信源 PATCH 会重建 pipeline 空壳，
    // 会把 bindings 引用抹掉造成数据丢失。改绑须走绑定感知路径（/catalog/bindings + 换绑作业）。
    if doc_has_binding_refs(&existing) {
        return Err(ApiError::precondition(
            "该信源含 AdapterBinding 引用，信源 PATCH 会丢失绑定；请经绑定 API 换绑",
        ));
    }
    let (mut doc, name) = source_doc(Some(&existing), map)?;
    // 归属项目固定；shared 可随 PATCH 调整。
    doc["project"] = existing
        .get("project")
        .cloned()
        .unwrap_or_else(|| json!(project.id()));
    doc["shared"] = json!(util::optional_flag(
        map,
        "shared",
        existing["shared"].as_bool().unwrap_or(false)
    )?);
    // 分类可随 PATCH 调整：缺省保持不变，空值退回未分组。
    if map.contains_key("groupId") {
        match group_id_of(map)? {
            Some(group_id) => {
                crate::source_groups::ensure_group(&state, &group_id, project.id()).await?;
                doc["groupId"] = json!(group_id);
            }
            None => {
                if let Some(map) = doc.as_object_mut() {
                    map.remove("groupId");
                }
            }
        }
    }
    config::update(
        &state.config_dir,
        &options(),
        &config::Draft {
            kind: "source".into(),
            slug: source_id.clone(),
            doc,
        },
    )?;
    state.services.config_reload()?;
    crate::observe::append_audit(
        &state,
        "catalog.source.update",
        "succeeded",
        "修改信源",
        json!({"id": source_id, "name": name}),
        None,
    )
    .await?;
    let row = source_by_slug(&state, &source_id).await?;
    Ok(util::ok(&row))
}

/// 读可选的 `groupId`：缺省或空串都表示未分组。
fn group_id_of(map: &Map<String, Value>) -> Result<Option<String>, ApiError> {
    Ok(util::optional_text(map, "groupId")?
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty()))
}

async fn source_by_slug(state: &AppState, slug: &str) -> Result<Value, ApiError> {
    source_rows(state)
        .await?
        .into_iter()
        .find(|row| row["id"] == slug)
        .ok_or_else(|| ApiError::not_found(format!("信源 {slug}")))
}

async fn delete_source(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    AxPath(source_id): AxPath<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let current = source_by_slug(&state, &source_id).await?;
    if !source_editable(&current, project.id()) {
        return Err(ApiError::not_found(format!("信源 {source_id}")));
    }
    util::if_match(&headers, current["etag"].as_str().unwrap_or(""))?;
    let operation = crate::operations::start(
        &state,
        "source.delete",
        vec![json!({"kind": "source", "id": source_id})],
        json!({"kind": "source.delete", "sourceId": source_id, "name": current["name"]}),
    )
    .await?;
    Ok(accepted(&operation))
}

pub async fn execute_source_delete(
    state: &AppState,
    operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let source_id = action["sourceId"].as_str().unwrap_or_default();
    config::remove(&state.config_dir, &options(), "source", source_id)?;
    state.services.config_reload()?;
    crate::observe::append_audit(
        state,
        "catalog.source.delete",
        "succeeded",
        "删除信源",
        json!({"id": source_id}),
        Some(Refs::op(operation_id)),
    )
    .await?;
    Ok(json!({"partial": false, "sourceId": source_id, "deleted": true}))
}

async fn enable_source(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    AxPath(source_id): AxPath<String>,
    headers: HeaderMap,
    Json(_body): Json<Value>,
) -> Result<Response, ApiError> {
    ensure_source_editable(&state, &source_id, project.id()).await?;
    toggle_source(&state, &source_id, true, &headers).await
}

async fn disable_source(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    AxPath(source_id): AxPath<String>,
    headers: HeaderMap,
    Json(_body): Json<Value>,
) -> Result<Response, ApiError> {
    ensure_source_editable(&state, &source_id, project.id()).await?;
    toggle_source(&state, &source_id, false, &headers).await
}

/// 切换信源共享：共享后其他项目可读（写仍限归属项目）。
async fn share_source(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    AxPath(source_id): AxPath<String>,
    Json(_body): Json<Value>,
) -> Result<Response, ApiError> {
    set_source_shared(&state, &source_id, project.id(), true).await
}

async fn unshare_source(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    AxPath(source_id): AxPath<String>,
    Json(_body): Json<Value>,
) -> Result<Response, ApiError> {
    set_source_shared(&state, &source_id, project.id(), false).await
}

/// 校验信源可被当前项目修改（归属校验），否则 404。
async fn ensure_source_editable(
    state: &AppState,
    source_id: &str,
    project_id: &str,
) -> Result<(), ApiError> {
    let row = source_by_slug(state, source_id).await?;
    if !source_editable(&row, project_id) {
        return Err(ApiError::not_found(format!("信源 {source_id}")));
    }
    Ok(())
}

/// 只改 shared 标记，重写信源正文（保留其余字段）。
async fn set_source_shared(
    state: &AppState,
    source_id: &str,
    project_id: &str,
    shared: bool,
) -> Result<Response, ApiError> {
    ensure_source_editable(state, source_id, project_id).await?;
    let existing = config::entries(&state.config_dir, "source")?
        .into_iter()
        .find(|entry| entry.slug == source_id)
        .map(|entry| entry.doc)
        .ok_or_else(|| ApiError::not_found(format!("信源 {source_id}")))?;
    let mut doc = existing;
    doc["shared"] = json!(shared);
    config::update(
        &state.config_dir,
        &options(),
        &config::Draft {
            kind: "source".into(),
            slug: source_id.to_string(),
            doc,
        },
    )?;
    state.services.config_reload()?;
    crate::observe::append_audit(
        state,
        "catalog.source.share",
        "succeeded",
        if shared {
            "共享信源"
        } else {
            "取消共享信源"
        },
        json!({"id": source_id, "shared": shared}),
        None,
    )
    .await?;
    let row = source_by_slug(state, source_id).await?;
    Ok(util::ok(&row))
}

async fn toggle_source(
    state: &AppState,
    source_id: &str,
    enabled: bool,
    headers: &HeaderMap,
) -> Result<Response, ApiError> {
    let current = source_by_slug(state, source_id).await?;
    util::if_match(headers, current["etag"].as_str().unwrap_or(""))?;
    config::toggle(&state.config_dir, &options(), "source", source_id, enabled)?;
    state.services.config_reload()?;
    crate::observe::append_audit(
        state,
        "catalog.source.toggle",
        "succeeded",
        if enabled {
            "启用信源"
        } else {
            "停工信源"
        },
        json!({"id": source_id, "enabled": enabled}),
        None,
    )
    .await?;
    let row = source_by_slug(state, source_id).await?;
    Ok(util::ok(&row))
}

async fn probe_source(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    AxPath(source_id): AxPath<String>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["locator", "persistResult"])?;
    let locator = util::optional_text(map, "locator")?;
    ensure_source_editable(&state, &source_id, project.id()).await?;
    let operation = crate::operations::start(
        &state,
        "catalog.probe",
        vec![json!({"kind": "source", "id": source_id})],
        json!({"kind": "source.probe", "sourceId": source_id, "locator": locator}),
    )
    .await?;
    Ok(accepted(&operation))
}

pub async fn execute_source_probe(
    state: &AppState,
    operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let source_id = action["sourceId"].as_str().unwrap_or_default();
    let locator = action["locator"].as_str().map(str::to_string);
    let task = state
        .services
        .task_create(
            &format!("source/{source_id}"),
            Some(strata_model::QueueName::Free),
            false,
            locator,
        )
        .await
        .map_err(ApiError::from)?;
    let task_id = task.task_id.as_str().to_string();
    crate::observe::append_event(
        state,
        "collection.probe",
        "连接探测已排队",
        json!({"sourceId": source_id, "taskId": task_id}),
        Refs {
            operation_id: Some(operation_id.to_string()),
            source_id: Some(source_id.to_string()),
            task_id: Some(task_id.clone()),
            ..Refs::default()
        },
    )
    .await?;
    Ok(json!({
        "partial": false,
        "sourceId": source_id,
        "taskId": task_id,
        "note": "探测作为一条手动采集任务执行，结果写入任务详情与日志。",
    }))
}

async fn list_access_methods(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let entries = config::entries(&state.config_dir, "accessor")?;
    let items: Vec<Value> = entries
        .into_iter()
        .map(|entry| {
            let implementation = entry.doc.get("implementation").and_then(Value::as_str);
            json!({
                "id": entry.slug,
                "name": business_method_name(implementation).unwrap_or_else(|| entry.doc.get("name").and_then(Value::as_str).unwrap_or(&entry.slug).to_string()),
                "description": entry.doc.get("description").cloned().unwrap_or(Value::Null),
                "enabled": entry.doc.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                "implementation": entry.doc.get("implementation").cloned().unwrap_or(Value::Null),
                "builtin": implementation.is_some_and(|value| value.starts_with("builtin:")),
                "references": Value::Null,
            })
        })
        .collect();
    list_static(&state, items, query, "access-methods")
}

/// 类型注册表只读投影（v3 R13）：与校验、写路径同源，前端表单/筛选据此驱动，禁止复刻矩阵。
async fn list_source_types(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let items = config::source_type::registry_json()["items"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    list_static(&state, items, query, "source-types")
}

async fn list_adapters(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let entries = config::entries(&state.config_dir, "adapter")?;
    let items: Vec<Value> = entries
        .into_iter()
        .map(|entry| {
            let implementation = entry.doc.get("implementation").and_then(Value::as_str);
            json!({
                "id": entry.slug,
                "name": business_adapter_name(implementation).unwrap_or_else(|| entry.doc.get("name").and_then(Value::as_str).unwrap_or(&entry.slug).to_string()),
                "description": entry.doc.get("description").cloned().unwrap_or(Value::Null),
                "enabled": entry.doc.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                "role": entry.doc.get("role").cloned().unwrap_or(Value::Null),
                "produces": entry.doc.get("produces").cloned().unwrap_or(Value::Null),
                "implementation": entry.doc.get("implementation").cloned().unwrap_or(Value::Null),
                "builtin": implementation.is_some_and(|value| value.starts_with("builtin:")),
            })
        })
        .collect();
    list_static(&state, items, query, "adapters")
}

async fn list_templates(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let templates = config::templates();
    let items: Vec<Value> = templates
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|template| {
            json!({
                "id": template["slug"],
                "kind": template["kind"],
                "name": template["name"],
                "description": template["description"],
                "doc": template["doc"],
            })
        })
        .collect();
    list_static(&state, items, query, "templates")
}

fn business_method_name(implementation: Option<&str>) -> Option<String> {
    let name = match implementation? {
        "builtin:rss" => "订阅源（RSS / Atom）",
        "builtin:http" => "网页抓取",
        "builtin:bundle-list" => "页面内嵌数据",
        "builtin:api" => "JSON 接口",
        "builtin:search" => "查询地址",
        "builtin:paid-api" => "付费接口",
        "builtin:push" => "推送接收",
        "builtin:browser" => "渲染页面",
        _ => return None,
    };
    Some(name.to_string())
}

fn business_adapter_name(implementation: Option<&str>) -> Option<String> {
    let name = match implementation? {
        "builtin:feed-items" => "订阅条目解析",
        "builtin:html-list" => "网页列表解析",
        "builtin:json-list" => "JSON 列表解析",
        "builtin:page-fields" => "详情字段补全",
        _ => return None,
    };
    Some(name.to_string())
}

fn list_static(
    _state: &AppState,
    items: Vec<Value>,
    query: ListQuery,
    filter_name: &str,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let filter = format!("{filter_name}:{}", query.query.clone().unwrap_or_default());
    let offset = util::page_offset(query.offset.as_deref(), query.cursor.as_deref(), &filter)?;
    let total = items.len();
    let items: Vec<Value> = items.into_iter().skip(offset).take(limit + 1).collect();
    let more = items.len() > limit;
    let items: Vec<Value> = items.into_iter().take(limit).collect();
    let next = if more || offset + items.len() < total {
        Some(cursor(&filter, offset + limit))
    } else {
        None
    };
    Ok(util::ok(&page_total(items, next, total)))
}

async fn create_source_analysis(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["url", "instruction", "modelId"])?;
    let url = util::uri(map, "url")?;
    let instruction = util::optional_text(map, "instruction")?;
    let model_id = util::optional_text(map, "modelId")?;
    let analysis_id = new_id("analysis");
    let doc = json!({
        "id": analysis_id,
        "url": url.to_string(),
        "instruction": instruction,
        "modelId": model_id,
        "status": "queued",
        "createdAt": now(),
    });
    state
        .store
        .insert_document("source_analysis", &analysis_id, &doc)
        .await?;
    let operation = crate::operations::start(
        &state,
        "catalog.sourceAnalysis",
        vec![json!({"kind": "sourceAnalysis", "id": analysis_id})],
        json!({"kind": "source.analysis", "analysisId": analysis_id, "url": url.to_string(), "instruction": instruction, "modelId": model_id}),
    )
    .await?;
    Ok(accepted(&operation))
}

pub async fn execute_source_analysis(
    state: &AppState,
    operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let analysis_id = action["analysisId"].as_str().unwrap_or_default();
    let url = action["url"].as_str().unwrap_or_default();
    let instruction = action["instruction"].as_str().unwrap_or("");
    let model_id = action["modelId"].as_str().filter(|value| !value.is_empty());
    let model = match model_id {
        Some(id) => crate::settings::model(state, id).await?,
        None => crate::settings::default_model(state)
            .await?
            .ok_or_else(|| ApiError::dependency("尚未配置默认模型，无法执行信源分析"))?,
    };
    let prompt = prompt_or_fallback(state, "prompt-source-analysis", &["v2", "v1"]).await?;
    let skill = skill_or_fallback(state, "skill-source-analysis", &["v2", "v1"]).await?;
    let input =
        format!("{prompt}\n\n技能约束：\n{skill}\n\n目标地址：{url}\n用户说明：{instruction}");
    let reply = crate::models::call_model(state, &model, &input, operation_id).await?;
    let raw = crate::models::extract_json(&reply)
        .ok_or_else(|| ApiError::dependency("模型没有返回可解析的信源草稿 JSON"))?;
    // R7：把模型产出统一成 v2 `{source, bindings[]}`；v1 裸信源标记 legacy 供人工确认前转换。
    let (proposal, legacy) = normalize_proposal(&raw)?;
    let doc = json!({
        "id": analysis_id,
        "url": url,
        "instruction": instruction,
        "modelId": model["id"],
        "status": "succeeded",
        "schemaVersion": 2,
        "legacy": legacy,
        "proposal": proposal,
        "reply": reply,
        "createdAt": now(),
    });
    if let Some(existing) = state
        .store
        .get_document("source_analysis", analysis_id)
        .await?
    {
        state
            .store
            .update_document("source_analysis", analysis_id, &doc, Some(existing.version))
            .await?;
    }
    crate::observe::append_event(
        state,
        "catalog.sourceAnalysis",
        "信源分析完成",
        json!({"analysisId": analysis_id, "url": url}),
        Refs {
            operation_id: Some(operation_id.to_string()),
            ..Refs::default()
        },
    )
    .await?;
    Ok(json!({
        "partial": false,
        "analysisId": analysis_id,
        "status": "succeeded",
        "proposal": proposal,
        "note": "草稿需要人工确认后通过新建信源提交。",
    }))
}

/// 解析提示词版本：优先 v2，缺失时回退到 v1（v1 只读兼容，R7）。
async fn prompt_or_fallback(
    state: &AppState,
    base: &str,
    versions: &[&str],
) -> Result<String, ApiError> {
    let mut last = None;
    for version in versions {
        let id = format!("{base}:v{}", version.trim_start_matches('v'));
        match crate::settings::prompt_content(state, &id).await {
            Ok(text) => return Ok(text),
            Err(error) => last = Some(error),
        }
    }
    Err(last.unwrap_or_else(|| ApiError::dependency("缺少可用的提示词版本")))
}

/// 解析 Skill 版本：优先 v2，缺失时回退到 v1（R7）。
async fn skill_or_fallback(
    state: &AppState,
    base: &str,
    versions: &[&str],
) -> Result<String, ApiError> {
    let mut last = None;
    for version in versions {
        let id = format!("{base}:v{}", version.trim_start_matches('v'));
        match crate::settings::skill_content(state, &id).await {
            Ok(text) => return Ok(text),
            Err(error) => last = Some(error),
        }
    }
    Err(last.unwrap_or_else(|| ApiError::dependency("缺少可用的 Skill 版本")))
}

/// R7：把 AI 产出统一成 v2 `{source, bindings[]}`。
///
/// - 已是 v2（含 `source` 对象）：原样透传，补齐缺失的 `bindings: []`，`legacy=false`。
/// - v1 存量形态（裸信源，含 `access` 或 `kind: source`）：把内联 pipeline 的
///   单适配器 + spec 步骤拆到 `bindings[]`，步骤改指 `bindingDraft` 下标，`legacy=true`。
///   这样旧 draft 不丢失，人工确认前完成形状升级。
pub(crate) fn normalize_proposal(raw: &Value) -> Result<(Value, bool), ApiError> {
    if raw.get("source").and_then(Value::as_object).is_some() {
        let source = stamp_source_type(&raw["source"]);
        let bindings = raw
            .get("bindings")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new()));
        return Ok((json!({"source": source, "bindings": bindings}), false));
    }
    let looks_like_source =
        raw.get("access").is_some() || raw.get("kind").and_then(Value::as_str) == Some("source");
    if looks_like_source {
        let (source, bindings) = split_inline_bindings(raw);
        let source = stamp_source_type(&source);
        return Ok((json!({"source": source, "bindings": bindings}), true));
    }
    Err(ApiError::dependency(
        "无法识别的信源草稿形状（既非 v1 裸信源，也非 v2 {source,bindings}）",
    ))
}

/// v3 R14/W2.4：给 AI 产出的信源草稿盖类型章——已声明则保留（写盘时仍过一致性/矩阵校验），
/// 缺失则按 access/pipeline 推导。与人工回填走同一 `derive`，不提供第二套推断。
fn stamp_source_type(source: &Value) -> Value {
    let mut doc = source.clone();
    let declared = doc
        .get("sourceType")
        .and_then(Value::as_str)
        .and_then(config::source_type::SourceType::parse);
    let kind = declared.unwrap_or_else(|| config::source_type::derive(&doc));
    if let Some(map) = doc.as_object_mut() {
        map.insert("sourceType".into(), json!(kind.as_str()));
    }
    doc
}

/// 从信源正文里剥离内联 pipeline spec 到 bindings（v1→v2）。
/// 只拆「单适配器 + 带 spec 参数」步骤；零参数与探针步骤原样保留（契合 R4/H6）。
fn split_inline_bindings(doc: &Value) -> (Value, Vec<Value>) {
    let mut source = doc.clone();
    let mut bindings: Vec<Value> = Vec::new();
    let Some(steps) = doc.get("pipeline").and_then(Value::as_array).cloned() else {
        return (source, bindings);
    };
    let mut new_steps: Vec<Value> = Vec::with_capacity(steps.len());
    for step in &steps {
        let Some(obj) = step.as_object() else {
            new_steps.push(step.clone());
            continue;
        };
        if obj.contains_key("run_if_accepts") || obj.contains_key("binding") {
            new_steps.push(step.clone());
            continue;
        }
        let run = obj
            .get("run")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let adapter = if run.len() == 1 {
            run[0].as_str().map(str::to_string)
        } else {
            None
        };
        let params = match (&adapter, obj.get("spec")) {
            (Some(a), Some(spec)) => spec.get(a).cloned().filter(Value::is_object),
            _ => None,
        };
        match params {
            Some(params) => {
                let adapter = adapter.unwrap_or_default();
                let index = bindings.len();
                bindings.push(json!({
                    "proposedName": format!("绑定 #{index}"),
                    "adapter": adapter,
                    "params": params,
                }));
                let mut ns = Map::new();
                for key in ["when", "for", "after"] {
                    if let Some(value) = obj.get(key) {
                        ns.insert(key.to_string(), value.clone());
                    }
                }
                ns.insert("bindingDraft".into(), Value::from(index));
                new_steps.push(Value::Object(ns));
            }
            None => new_steps.push(step.clone()),
        }
    }
    source["pipeline"] = Value::Array(new_steps);
    (source, bindings)
}

async fn list_adapter_drafts(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let docs = state.store.list_documents("adapter_draft").await?;
    let items: Vec<Value> = docs
        .into_iter()
        .map(|doc| {
            let mut body = doc.body;
            if let Some(map) = body.as_object_mut() {
                map.insert(
                    "etag".into(),
                    Value::String(format!("adapter-draft-{}-v{}", doc.id, doc.version)),
                );
            }
            body
        })
        .collect();
    list_static(&state, items, query, "adapter-drafts")
}

async fn create_adapter_draft(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["sourceAnalysisId", "name", "instruction"])?;
    let analysis_id = util::non_empty(map, "sourceAnalysisId")?;
    let name = util::non_empty(map, "name")?;
    let instruction = util::optional_text(map, "instruction")?;
    state
        .store
        .get_document("source_analysis", &analysis_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("信源分析 {analysis_id}")))?;
    let draft_id = new_id("adapter");
    let doc = json!({
        "id": draft_id,
        "name": name,
        "sourceAnalysisId": analysis_id,
        "instruction": instruction,
        "status": "queued",
        "createdAt": now(),
    });
    state
        .store
        .insert_document("adapter_draft", &draft_id, &doc)
        .await?;
    let operation = crate::operations::start(
        &state,
        "catalog.adapterDraft",
        vec![json!({"kind": "adapterDraft", "id": draft_id})],
        json!({"kind": "adapter.draft", "draftId": draft_id}),
    )
    .await?;
    Ok(accepted(&operation))
}

pub async fn execute_adapter_draft(
    state: &AppState,
    operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let draft_id = action["draftId"].as_str().unwrap_or_default();
    let draft = state
        .store
        .get_document("adapter_draft", draft_id)
        .await?
        .ok_or_else(|| ApiError::not_found(draft_id))?;
    let analysis_id = draft.body["sourceAnalysisId"].as_str().unwrap_or_default();
    let analysis = state
        .store
        .get_document("source_analysis", analysis_id)
        .await?
        .ok_or_else(|| ApiError::not_found(analysis_id))?;
    let model = crate::settings::default_model(state)
        .await?
        .ok_or_else(|| ApiError::dependency("尚未配置默认模型，无法生成适配器"))?;
    let prompt = crate::settings::prompt_content(state, "prompt-adapter-draft:v1").await?;
    let skill = crate::settings::skill_content(state, "skill-adapter-generation:v1").await?;
    let input = format!(
        "{prompt}\n\n技能约束：\n{skill}\n\n站点分析：{}\n适配器名称：{}\n补充说明：{}",
        analysis.body["proposal"],
        draft.body["name"].as_str().unwrap_or(""),
        draft.body["instruction"].as_str().unwrap_or("")
    );
    let reply = crate::models::call_model(state, &model, &input, operation_id).await?;
    let generated = crate::models::extract_json(&reply)
        .ok_or_else(|| ApiError::dependency("模型没有返回可解析的适配器草稿 JSON"))?;
    let check = validate_adapter_draft(&generated);
    let status = if check.is_empty() {
        "validated"
    } else {
        "validationFailed"
    };
    let mut body = draft.body.clone();
    body["status"] = Value::String(status.into());
    body["generated"] = generated;
    body["checks"] = json!(check);
    body["reply"] = Value::String(reply);
    body["updatedAt"] = Value::String(now());
    state
        .store
        .update_document("adapter_draft", draft_id, &body, Some(draft.version))
        .await?;
    crate::observe::append_event(
        state,
        "catalog.adapterDraft",
        "适配器草稿已生成",
        json!({"draftId": draft_id, "status": status, "checks": check}),
        Refs {
            operation_id: Some(operation_id.to_string()),
            ..Refs::default()
        },
    )
    .await?;
    Ok(json!({
        "partial": false,
        "draftId": draft_id,
        "status": status,
        "checks": check,
        "note": "通过静态检查后仍需人工批准才会写入配置。",
    }))
}

fn validate_adapter_draft(value: &Value) -> Vec<Value> {
    let mut issues = Vec::new();
    let Some(map) = value.as_object() else {
        issues.push(json!({"code": "shape", "message": "草稿必须是 JSON 对象"}));
        return issues;
    };
    if map.get("implementation").and_then(Value::as_str).is_none() {
        issues.push(json!({"code": "implementation", "message": "缺少 implementation"}));
    }
    if map.get("spec").and_then(Value::as_object).is_none() {
        issues.push(json!({"code": "spec", "message": "缺少 spec"}));
    }
    let text = value.to_string().to_lowercase();
    for banned in [
        "127.0.0.1",
        "localhost",
        "javascript:",
        "eval(",
        "process.env",
    ] {
        if text.contains(banned) {
            issues.push(json!({"code": "forbidden", "message": format!("包含禁止内容: {banned}")}));
        }
    }
    if let Some(url) = map
        .get("spec")
        .and_then(|spec| spec.get("url"))
        .and_then(Value::as_str)
        && url::Url::parse(url).is_ok_and(|parsed| {
            parsed
                .host_str()
                .is_some_and(|host| host == "127.0.0.1" || host == "localhost" || host == "::1")
        })
    {
        issues.push(json!({"code": "ssrf", "message": "不得指向回环地址"}));
    }
    issues
}

async fn approve_adapter_draft(
    State(state): State<Shared>,
    AxPath(draft_id): AxPath<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["validationId", "comment"])?;
    let comment = util::non_empty(map, "comment")?;
    let draft = state
        .store
        .get_document("adapter_draft", &draft_id)
        .await?
        .ok_or_else(|| ApiError::not_found(&draft_id))?;
    util::if_match(
        &headers,
        &format!("adapter-draft-{draft_id}-v{}", draft.version),
    )?;
    if draft.body["status"] != "validated" {
        return Err(ApiError::conflict("草稿尚未通过校验，不能批准"));
    }
    let operation = crate::operations::start(
        &state,
        "catalog.adapterApprove",
        vec![json!({"kind": "adapterDraft", "id": draft_id})],
        json!({"kind": "adapter.approve", "draftId": draft_id, "comment": comment}),
    )
    .await?;
    Ok(accepted(&operation))
}

pub async fn execute_adapter_approve(
    state: &AppState,
    operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let draft_id = action["draftId"].as_str().unwrap_or_default();
    let draft = state
        .store
        .get_document("adapter_draft", draft_id)
        .await?
        .ok_or_else(|| ApiError::not_found(draft_id))?;
    let generated = draft.body["generated"].clone();
    let name = draft.body["name"].as_str().unwrap_or("自定义适配器");
    let slug = format!("{}-{}", util::slug(name), util::hash_short(draft_id));
    let doc = generated.as_object().cloned().unwrap_or_default();
    let mut doc = Value::Object(doc);
    if let Some(map) = doc.as_object_mut() {
        map.insert("name".into(), Value::String(name.into()));
        map.insert("kind".into(), Value::String("adapter".into()));
    }
    config::create(
        &state.config_dir,
        &options(),
        &config::Draft {
            kind: "adapter".into(),
            slug: slug.clone(),
            doc,
        },
    )?;
    state.services.config_reload()?;
    let mut body = draft.body.clone();
    body["status"] = Value::String("approved".into());
    body["approvedAt"] = Value::String(now());
    body["adapterId"] = Value::String(slug.clone());
    state
        .store
        .update_document("adapter_draft", draft_id, &body, Some(draft.version))
        .await?;
    crate::observe::append_audit(
        state,
        "catalog.adapter.approve",
        "succeeded",
        "批准适配器草稿",
        json!({"draftId": draft_id, "adapterId": slug, "comment": action["comment"]}),
        Some(Refs::op(operation_id)),
    )
    .await?;
    Ok(json!({"partial": false, "draftId": draft_id, "adapterId": slug, "status": "approved"}))
}

async fn reject_adapter_draft(
    State(state): State<Shared>,
    AxPath(draft_id): AxPath<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["decision", "comment", "selectedOutputIds"])?;
    let comment = util::optional_text(map, "comment")?.unwrap_or_default();
    if let Some(decision) = map.get("decision").and_then(Value::as_str)
        && decision != "reject"
    {
        return Err(ApiError::validation(
            "decision",
            "enum",
            "适配器草稿只支持 reject",
        ));
    }
    let draft = state
        .store
        .get_document("adapter_draft", &draft_id)
        .await?
        .ok_or_else(|| ApiError::not_found(&draft_id))?;
    util::if_match(
        &headers,
        &format!("adapter-draft-{draft_id}-v{}", draft.version),
    )?;
    let mut body = draft.body.clone();
    body["status"] = Value::String("rejected".into());
    body["rejectedAt"] = Value::String(now());
    body["rejectReason"] = Value::String(comment);
    state
        .store
        .update_document("adapter_draft", &draft_id, &body, Some(draft.version))
        .await?;
    crate::observe::append_audit(
        &state,
        "catalog.adapter.reject",
        "succeeded",
        "驳回适配器草稿",
        json!({"draftId": draft_id}),
        None,
    )
    .await?;
    Ok(util::ok(&body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalize_upgrades_v1_bare_source_and_splits_bindings() {
        let v1 = json!({
            "name": "Demo",
            "access": {"accessor": "accessor/rss", "locator": {"url": "https://example.com/feed"}},
            "pipeline": [{"run": ["adapter/json-list"], "spec": {"adapter/json-list": {"items": "/data"}}}],
            "schedule": {"interval": "1h"}
        });
        let (proposal, legacy) = normalize_proposal(&v1).expect("v1 可识别");
        assert!(legacy, "v1 裸信源应标记 legacy");
        assert_eq!(proposal["bindings"].as_array().unwrap().len(), 1);
        assert_eq!(
            proposal["bindings"][0]["adapter"],
            json!("adapter/json-list")
        );
        assert_eq!(proposal["bindings"][0]["params"]["items"], json!("/data"));
        // 信源侧只剩声明，spec 被移出并改指 bindingDraft 下标。
        let step = &proposal["source"]["pipeline"][0];
        assert!(step.get("spec").is_none(), "内联 spec 应被移出信源");
        assert_eq!(step["bindingDraft"], json!(0));
    }

    #[test]
    fn normalize_passes_through_v2_shape() {
        let v2 = json!({
            "source": {"name": "Demo", "access": {"accessor": "accessor/rss", "locator": {"url": "https://e.com/f"}}},
            "bindings": [{"name": "b", "adapter": "adapter/json-list", "params": {"items": "/x"}}]
        });
        let (proposal, legacy) = normalize_proposal(&v2).expect("v2 可识别");
        assert!(!legacy, "v2 不应标记 legacy");
        assert_eq!(proposal["bindings"].as_array().unwrap().len(), 1);
        assert_eq!(proposal["source"]["name"], json!("Demo"));
    }

    #[test]
    fn normalize_v2_without_bindings_gets_empty_list() {
        let v2 = json!({"source": {"name": "Demo", "access": {}}});
        let (proposal, legacy) = normalize_proposal(&v2).unwrap();
        assert!(!legacy);
        assert_eq!(proposal["bindings"], json!([]));
    }

    #[test]
    fn normalize_rejects_unknown_shape() {
        let junk = json!({"foo": "bar"});
        assert!(normalize_proposal(&junk).is_err());
    }

    #[test]
    fn normalize_keeps_zero_param_and_probe_steps_inline() {
        let v1 = json!({
            "access": {"accessor": "accessor/rss", "locator": {"url": "https://e.com/f"}},
            "pipeline": [
                {"run": ["adapter/feed-items"]},
                {"run_if_accepts": ["adapter/probe"]}
            ]
        });
        let (proposal, legacy) = normalize_proposal(&v1).unwrap();
        assert!(legacy);
        // 零参数与探针步骤都不拆。
        assert_eq!(proposal["bindings"], json!([]));
        assert_eq!(
            proposal["source"]["pipeline"][0]["run"],
            json!(["adapter/feed-items"])
        );
    }

    /// v3 W2.1：缺省声明时 source_doc 按装配形状推导类型并写入。
    #[test]
    fn source_doc_derives_type_when_absent() {
        let input = json!({
            "name": "N", "accessMethodId": "rss", "adapterIds": ["feed-items"],
            "enabled": true, "endpoint": "https://e.com/feed"
        });
        let (doc, _) = source_doc(None, input.as_object().unwrap()).expect("装配成功");
        assert_eq!(doc["sourceType"], json!("feed"));
    }

    /// v3 W2.1/S3：未知类型值被拒。
    #[test]
    fn source_doc_rejects_unknown_type() {
        let input = json!({
            "name": "N", "accessMethodId": "rss", "adapterIds": ["feed-items"],
            "enabled": true, "endpoint": "https://e.com/feed", "sourceType": "nonsense"
        });
        assert!(source_doc(None, input.as_object().unwrap()).is_err());
    }

    /// v3 W2.1：显式声明与形状不一致被拒。
    #[test]
    fn source_doc_rejects_mismatched_type() {
        let input = json!({
            "name": "N", "accessMethodId": "rss", "adapterIds": ["feed-items"],
            "enabled": true, "endpoint": "https://e.com/feed", "sourceType": "web"
        });
        assert!(source_doc(None, input.as_object().unwrap()).is_err());
    }

    /// v3 W2.1：矩阵违规（feed 形塞入 page-fields）即便未声明类型也被拒。
    #[test]
    fn source_doc_rejects_matrix_violation() {
        let input = json!({
            "name": "N", "accessMethodId": "rss", "adapterIds": ["page-fields"],
            "enabled": true, "endpoint": "https://e.com/feed"
        });
        assert!(source_doc(None, input.as_object().unwrap()).is_err());
    }

    /// v3 W3.3：接收类靠频道标识落盘，不带地址、不留轮询计划。
    #[test]
    fn source_doc_accepts_push_channel() {
        let input = json!({
            "name": "推送", "accessMethodId": "push", "adapterIds": ["json-list"],
            "enabled": true, "sourceType": "push", "channel": " inbox ",
            "schedule": {"mode": "fixed", "intervalMs": 3_600_000}
        });
        let (doc, _) = source_doc(None, input.as_object().unwrap()).unwrap();
        assert_eq!(doc["sourceType"], json!("push"));
        assert_eq!(
            doc.pointer("/access/locator/channel"),
            Some(&json!("inbox"))
        );
        assert!(doc.pointer("/access/locator/url").is_none());
        assert!(doc.get("schedule").is_none());
    }

    /// v3 W3.3：接收类缺频道必须拒绝，而不是落一份采集器无法绑定的声明。
    #[test]
    fn source_doc_requires_channel_for_push() {
        let input = json!({
            "name": "推送", "accessMethodId": "push", "adapterIds": ["json-list"],
            "enabled": true, "sourceType": "push"
        });
        assert!(source_doc(None, input.as_object().unwrap()).is_err());
    }

    /// 频道与 url 形态互斥：带 channel 的声明不能再派生为 url 源。
    #[test]
    fn source_doc_channel_overrides_endpoint() {
        let input = json!({
            "name": "推送", "accessMethodId": "push", "adapterIds": ["json-list"],
            "enabled": true, "channel": "inbox", "endpoint": "https://e.com/x"
        });
        let (doc, _) = source_doc(None, input.as_object().unwrap()).unwrap();
        assert_eq!(doc["sourceType"], json!("push"));
        assert!(doc.pointer("/access/locator/url").is_none());
    }

    /// v3 W2.4：AI 草稿缺类型时由同一 derive 盖章。
    #[test]
    fn normalize_stamps_derived_type() {
        let v2 = json!({
            "source": {
                "access": {"accessor": "accessor/bundle-list", "locator": {"url": "https://e.com"}},
                "pipeline": [{"run": ["adapter/json-list"]}]
            }
        });
        let (proposal, _) = normalize_proposal(&v2).unwrap();
        assert_eq!(proposal["source"]["sourceType"], json!("web"));
    }
}
