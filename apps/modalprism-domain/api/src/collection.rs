//! 采集计划与采集运行：自动与手动共用同一条任务创建路径。

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Value, json};
use strata_client::TaskFilter;
use strata_model::QueueName;

use crate::error::ApiError;
use crate::operations::accepted;
use crate::util::{self, cursor, page_total};
use crate::{AppState, Shared};

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/collection/plans", get(list_plans).post(create_plan))
        .route(
            "/collection/plans/{planId}",
            get(get_plan).patch(update_plan).delete(delete_plan),
        )
        .route("/collection/plans/{planId}/pause", post(pause_plan))
        .route("/collection/plans/{planId}/resume", post(resume_plan))
        .route("/collection/jobs", post(create_job))
        .route("/collection/runs", get(list_runs).post(create_run))
        .route("/collection/runs/{runId}", get(get_run))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListQuery {
    #[serde(default)]
    limit: Option<String>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    offset: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    source_id: Option<String>,
    /// 采集计划：只看该计划派发的运行。
    #[serde(default)]
    plan_id: Option<String>,
    /// `all` 取消项目隔离过滤（管理与迁移用）。
    #[serde(default)]
    scope: Option<String>,
}

pub(crate) async fn plan_rows(state: &AppState) -> Result<Vec<Value>, ApiError> {
    let list = config::list(&state.config_dir)?;
    let sources = crate::catalog::source_rows(state).await?;
    // 计划维度的投递目标存在内容投递域：按计划 id 回填，计划与投递只单向关联。
    let delivery = crate::delivery::plan_rule_views(state).await?;
    let source_status = |id: &str| -> Option<Value> {
        sources
            .iter()
            .find(|row| row["id"].as_str() == Some(id))
            .cloned()
    };
    let mut plans = Vec::new();
    for plan in list["plans"].as_array().cloned().unwrap_or_default() {
        let slug = plan["slug"].as_str().unwrap_or_default().to_string();
        let targets: Vec<String> = plan["targets"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| {
                value
                    .as_str()
                    .map(|id| id.trim_start_matches("source/").to_string())
            })
            .collect();
        let enabled = plan["enabled"].as_bool().unwrap_or(true);
        // 计划归属：声明优先，其次按首个目标信源的归属派生，最后回落默认项目。
        let project = plan_project(&plan, &targets, &sources);
        // 运行态借首个目标信源的调度视图（节律按目标数复制派发，视图取其一即足）。
        let first = targets.first().and_then(|id| source_status(id));
        let status = if !enabled {
            "paused"
        } else if first
            .as_ref()
            .and_then(|row| row["enabled"].as_bool())
            .unwrap_or(true)
        {
            "active"
        } else {
            "disabled"
        };
        let etag = util::etag(&[&slug, &util::hash_short(&plan["doc"].to_string())]);
        let rule = delivery.get(&slug).cloned().unwrap_or(Value::Null);
        plans.push(json!({
            "id": slug,
            "name": plan["name"],
            "description": plan["description"],
            "project": project,
            "targets": targets,
            "trigger": plan["trigger"],
            "queue": plan["queue"],
            "priority": plan["priority"],
            "quota": plan["quota"],
            "stopPolicy": plan["stopPolicy"],
            "enabled": enabled,
            "status": status,
            "health": first.as_ref().map(|row| row["health"].clone()).unwrap_or(Value::Null),
            "nextRunAt": first.as_ref().map(|row| row["nextRunAt"].clone()).unwrap_or(Value::Null),
            "lastRunAt": first.as_ref().map(|row| row["lastRunAt"].clone()).unwrap_or(Value::Null),
            "todayYield": first.as_ref().map(|row| row["todayYield"].clone()).unwrap_or(Value::Null),
            "deliveryRuleId": rule["ruleId"].clone(),
            "deliveryTargets": rule["targets"].clone(),
            "deliveryEnabled": rule["enabled"].clone(),
            "deliveryLastRunAt": rule["lastRunAt"].clone(),
            "etag": etag,
        }));
    }
    Ok(plans)
}

/// 计划归属：声明优先，其次按首个目标信源的归属派生，最后回落默认项目。
/// 计划自身不存 project 的旧数据也能落到正确的项目，不需要一次性迁移。
fn plan_project(plan: &Value, targets: &[String], sources: &[Value]) -> String {
    if let Some(project) = plan
        .get("project")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        return project.to_string();
    }
    for target in targets {
        if let Some(row) = sources
            .iter()
            .find(|row| row["id"].as_str() == Some(target.as_str()))
            && let Some(project) = row["project"].as_str()
        {
            return project.to_string();
        }
    }
    crate::projects::DEFAULT_PROJECT_ID.to_string()
}

/// 取属于当前项目的计划；他项目的计划一律按不存在处理，不泄露存在性。
async fn plan_row_owned(
    state: &AppState,
    plan_id: &str,
    project_id: &str,
) -> Result<Value, ApiError> {
    let plan = plan_rows(state)
        .await?
        .into_iter()
        .find(|plan| plan["id"] == plan_id)
        .ok_or_else(|| ApiError::not_found(format!("采集计划 {plan_id}")))?;
    if plan["project"].as_str() != Some(project_id) {
        return Err(ApiError::not_found(format!("采集计划 {plan_id}")));
    }
    Ok(plan)
}

/// 计划只能引用归属当前项目的信源：共享信源只放开读，不参与他项目的调度。
async fn ensure_targets_owned(
    state: &AppState,
    project_id: &str,
    targets: &[String],
) -> Result<(), ApiError> {
    let rows = crate::catalog::source_rows(state).await?;
    for target in targets {
        let id = target.trim_start_matches("source/");
        let row = rows
            .iter()
            .find(|row| row["id"].as_str() == Some(id))
            .ok_or_else(|| ApiError::validation("targets", "notFound", format!("信源 {id} 不存在")))?;
        if !crate::catalog::source_editable(row, project_id) {
            return Err(ApiError::validation(
                "targets",
                "forbidden",
                format!("信源 {id} 不属于当前项目"),
            ));
        }
    }
    Ok(())
}

/// 当前项目自有（非共享）的信源集合；`scope=all` 时返回 None（不过滤）。
/// 运行记录属于采集产物，只能跟自有信源走：共享信源只读声明，不带运行数据。
async fn visible_sources_opt(
    state: &AppState,
    project: &crate::ActiveProject,
    scope: Option<&str>,
) -> Result<Option<std::collections::HashSet<String>>, ApiError> {
    if scope == Some("all") {
        Ok(None)
    } else {
        Ok(Some(
            crate::catalog::owned_source_ids(state, project.id()).await?,
        ))
    }
}

async fn list_plans(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let filter = format!(
        "plans:{}:{}",
        query.status.clone().unwrap_or_default(),
        project.id()
    );
    let offset = util::page_offset(query.offset.as_deref(), query.cursor.as_deref(), &filter)?;
    let mut plans = plan_rows(&state).await?;
    // 项目隔离：计划归属按声明的 project（或目标信源归属）判定；scope=all 取消过滤。
    if query.scope.as_deref() != Some("all") {
        let active = project.id();
        plans.retain(|plan| plan["project"].as_str() == Some(active));
    }
    if let Some(status) = query.status.as_deref().filter(|value| !value.is_empty()) {
        plans.retain(|plan| plan["status"] == status);
    }
    let total = plans.len();
    let items: Vec<Value> = plans.into_iter().skip(offset).take(limit + 1).collect();
    let more = items.len() > limit;
    let items: Vec<Value> = items.into_iter().take(limit).collect();
    let next = if more || offset + items.len() < total {
        Some(cursor(&filter, offset + limit))
    } else {
        None
    };
    Ok(util::ok(&page_total(items, next, total)))
}

async fn get_plan(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Path(plan_id): Path<String>,
) -> Result<Response, ApiError> {
    let plan = plan_row_owned(&state, &plan_id, project.id()).await?;
    let mut response = util::ok(&plan);
    if let Ok(value) = plan["etag"].as_str().unwrap_or("").parse() {
        response
            .headers_mut()
            .insert(axum::http::header::ETAG, value);
    }
    Ok(response)
}

/// 由目标信源与请求内容派生一个合法 plan slug（slug 只允许字母数字与 `._-`）。
fn derive_plan_slug(body: &serde_json::Map<String, Value>, targets: &[String]) -> String {
    if let Some(id) = body.get("id").and_then(Value::as_str) {
        return id.trim_start_matches("plan/").to_string();
    }
    let first = targets
        .first()
        .map(|id| id.trim_start_matches("source/").to_string())
        .unwrap_or_else(|| "plan".to_string());
    format!(
        "{first}-{}",
        util::hash_short(&Value::Object(body.clone()).to_string())
    )
}

fn extract_targets(map: &serde_json::Map<String, Value>) -> Vec<String> {
    let key = if map.contains_key("targets") {
        "targets"
    } else {
        "sourceIds"
    };
    map.get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| value.as_str().map(str::to_string))
        .map(|id| {
            if id.starts_with("source/") {
                id
            } else {
                format!("source/{id}")
            }
        })
        .collect()
}

async fn create_plan(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(
        map,
        &[
            "id",
            "name",
            "description",
            "targets",
            "sourceIds",
            "trigger",
            "queue",
            "priority",
            "quota",
            "stopPolicy",
            "enabled",
            "deliveryTargets",
            "deliveryEnabled",
        ],
    )?;
    let targets = extract_targets(map);
    if targets.is_empty() {
        return Err(ApiError::validation(
            "targets",
            "required",
            "计划至少要有一个信源",
        ));
    }
    // 归属校验：跨项目引用会被写进计划，必须在落盘前拒绝。
    ensure_targets_owned(&state, project.id(), &targets).await?;
    let trigger = map
        .get("trigger")
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| json!({"kind": "recurring", "interval": "1h"}));
    let slug = derive_plan_slug(map, &targets);
    let mut doc = json!({
        "id": format!("plan/{slug}"),
        "kind": "plan",
        "name": map.get("name").cloned().unwrap_or(json!("采集计划")),
        "project": project.id(),
        "targets": targets,
        "trigger": trigger,
    });
    if let Some(object) = doc.as_object_mut() {
        for key in [
            "description",
            "queue",
            "priority",
            "quota",
            "stopPolicy",
            "enabled",
        ] {
            if let Some(value) = map.get(key) {
                object.insert(key.to_string(), value.clone());
            }
        }
    }
    config::create(
        &state.config_dir,
        &config::LoadOptions::from_process(),
        &config::Draft {
            kind: "plan".into(),
            slug: slug.clone(),
            doc,
        },
    )?;
    state.services.config_reload()?;
    crate::delivery::sync_plan_rule(
        &state,
        &slug,
        map.get("name").and_then(Value::as_str).unwrap_or("采集计划"),
        map.get("deliveryTargets"),
        map.get("deliveryEnabled").and_then(Value::as_bool),
    )
    .await?;
    crate::observe::append_audit(
        &state,
        "collection.plan.create",
        "succeeded",
        "新建采集计划",
        json!({"planId": slug}),
        None,
    )
    .await?;
    let plan = plan_rows(&state)
        .await?
        .into_iter()
        .find(|plan| plan["id"] == slug)
        .ok_or_else(|| ApiError::not_found(format!("采集计划 {slug}")))?;
    Ok(util::ok(&plan))
}

async fn delete_plan(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Path(plan_id): Path<String>,
) -> Result<Response, ApiError> {
    plan_row_owned(&state, &plan_id, project.id()).await?;
    config::remove(
        &state.config_dir,
        &config::LoadOptions::from_process(),
        "plan",
        &plan_id,
    )?;
    state.services.config_reload()?;
    // 计划没了，它名下的自动投递规则也随之失效，一并清掉避免孤儿。
    crate::delivery::remove_plan_rule(&state, &plan_id).await?;
    crate::observe::append_audit(
        &state,
        "collection.plan.delete",
        "succeeded",
        "删除采集计划",
        json!({"planId": plan_id}),
        None,
    )
    .await?;
    Ok(util::ok(&json!({"id": plan_id, "deleted": true})))
}

/// 批量作业：一次多信源单次采集。建一份临时 oneShot 计划留痕，并即时派发各目标任务。
async fn create_job(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["targets", "sourceIds", "name", "queue", "priority"])?;
    let targets = extract_targets(map);
    if targets.is_empty() {
        return Err(ApiError::validation(
            "targets",
            "required",
            "作业至少要有一个信源",
        ));
    }
    ensure_targets_owned(&state, project.id(), &targets).await?;
    let slug = derive_plan_slug(map, &targets);
    let doc = json!({
        "id": format!("plan/{slug}"),
        "kind": "plan",
        "name": map.get("name").cloned().unwrap_or(json!("一次性采集作业")),
        "project": project.id(),
        "targets": targets,
        "trigger": {"kind": "oneShot", "fireAt": "immediate"},
        // 作业计划仅留痕审计：即时 task_create 已派发，禁用可避免调度轮次二次派发同一 oneShot。
        "enabled": false,
    });
    // 临时作业计划落盘留痕，便于审计；派发即时、不参与后续定时节律。
    config::create(
        &state.config_dir,
        &config::LoadOptions::from_process(),
        &config::Draft {
            kind: "plan".into(),
            slug: slug.clone(),
            doc,
        },
    )?;
    state.services.config_reload()?;
    let queue = match map.get("queue").and_then(Value::as_str) {
        Some("paid") => Some(QueueName::Paid),
        Some("render") => Some(QueueName::Render),
        Some("push") => Some(QueueName::Push),
        Some("free") => Some(QueueName::Free),
        _ => None,
    };
    let mut dispatched = Vec::new();
    let mut failed = Vec::new();
    for target in &targets {
        match state.services.task_create(target, queue, true, None).await {
            Ok(row) => dispatched
                .push(json!({"sourceId": target, "taskId": row.task_id, "status": row.status})),
            Err(error) => failed.push(json!({"sourceId": target, "error": error.to_string()})),
        }
    }
    crate::observe::append_audit(
        &state,
        "collection.job.create",
        if failed.is_empty() {
            "succeeded"
        } else {
            "partial"
        },
        "批量采集作业",
        json!({"planId": slug, "dispatched": dispatched.len(), "failed": failed.len()}),
        None,
    )
    .await?;
    Ok(util::ok(&json!({
        "planId": slug,
        "dispatched": dispatched,
        "failed": failed,
    })))
}

async fn update_plan(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Path(plan_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(
        map,
        &[
            "trigger",
            "schedule",
            "quota",
            "stopPolicy",
            "priority",
            "queue",
            "enabled",
            "targets",
            "name",
            "description",
            "deliveryTargets",
            "deliveryEnabled",
        ],
    )?;
    let plan = plan_row_owned(&state, &plan_id, project.id()).await?;
    util::if_match(&headers, plan["etag"].as_str().unwrap_or(""))?;
    // 改目标也要走归属校验：不能把他项目信源拉进自己的计划。
    if map.contains_key("targets") {
        ensure_targets_owned(&state, project.id(), &extract_targets(map)).await?;
    }
    let entry = config::entries(&state.config_dir, "plan")?
        .into_iter()
        .find(|entry| entry.slug == plan_id)
        .ok_or_else(|| ApiError::not_found(format!("采集计划 {plan_id}")))?;
    let mut doc = entry.doc.as_object().cloned().unwrap_or_default();
    // 兼容前端仍在编辑的 `schedule`：把 intervalMs/mode 翻译成计划的 trigger（节律已归计划）。
    if let Some(schedule) = util::optional_object(map, "schedule")? {
        let mut next = doc
            .get("trigger")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mode = schedule
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("fixed");
        if mode == "push" {
            next.insert("kind".into(), json!("oneShot"));
        } else {
            next.insert("kind".into(), json!("recurring"));
            if let Some(interval_ms) = schedule.get("intervalMs").and_then(Value::as_i64) {
                next.insert("interval".into(), json!(util::duration_text(interval_ms)));
            }
            if let Some(floor) = schedule.get("minimumIntervalMs").and_then(Value::as_i64) {
                next.insert("floor".into(), json!(util::duration_text(floor)));
            }
        }
        doc.insert("trigger".into(), Value::Object(next));
    }
    // trigger 至少要有 kind；recurring 缺 interval 时回落到系统默认间隔。
    if let Some(trigger) = util::optional_object(map, "trigger")? {
        let mut next = doc
            .get("trigger")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (key, value) in trigger {
            next.insert(key, value);
        }
        next.entry(String::from("kind"))
            .or_insert_with(|| json!("recurring"));
        if next.get("kind").and_then(Value::as_str) == Some("recurring")
            && next.get("interval").is_none()
        {
            let default_interval = crate::settings::value(&state, "collection.defaultIntervalMs")
                .await?
                .as_i64()
                .unwrap_or(1_800_000);
            let minimum = crate::settings::value(&state, "collection.minimumIntervalMs")
                .await?
                .as_i64()
                .unwrap_or(60_000)
                .max(1_000);
            let requested_floor = next
                .get("minimumIntervalMs")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let floor = minimum.max(requested_floor);
            if default_interval < floor {
                return Err(ApiError::validation(
                    "trigger.interval",
                    "outOfRange",
                    format!("间隔不能低于系统最小间隔 {floor}ms"),
                ));
            }
            next.insert(
                "interval".into(),
                json!(util::duration_text(default_interval)),
            );
        }
        doc.insert("trigger".into(), Value::Object(next));
    }
    for key in [
        "name",
        "description",
        "quota",
        "stopPolicy",
        "priority",
        "queue",
        "enabled",
        "targets",
    ] {
        if let Some(value) = map.get(key) {
            doc.insert(key.into(), value.clone());
        }
    }
    config::update(
        &state.config_dir,
        &config::LoadOptions::from_process(),
        &config::Draft {
            kind: "plan".into(),
            slug: plan_id.clone(),
            doc: Value::Object(doc),
        },
    )?;
    state.services.config_reload()?;
    crate::delivery::sync_plan_rule(
        &state,
        &plan_id,
        plan["name"].as_str().unwrap_or(""),
        map.get("deliveryTargets"),
        map.get("deliveryEnabled").and_then(Value::as_bool),
    )
    .await?;
    crate::observe::append_audit(
        &state,
        "collection.plan.update",
        "succeeded",
        "调整采集计划",
        json!({"planId": plan_id, "trigger": map.get("trigger")}),
        None,
    )
    .await?;
    let plan = plan_rows(&state)
        .await?
        .into_iter()
        .find(|plan| plan["id"] == plan_id)
        .ok_or_else(|| ApiError::not_found(format!("采集计划 {plan_id}")))?;
    Ok(util::ok(&plan))
}

async fn pause_plan(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Path(plan_id): Path<String>,
    headers: HeaderMap,
    Json(_body): Json<Value>,
) -> Result<Response, ApiError> {
    let plan = plan_row_owned(&state, &plan_id, project.id()).await?;
    util::if_match(&headers, plan["etag"].as_str().unwrap_or(""))?;
    config::toggle(
        &state.config_dir,
        &config::LoadOptions::from_process(),
        "plan",
        &plan_id,
        false,
    )?;
    state.services.config_reload()?;
    crate::observe::append_audit(
        &state,
        "collection.plan.pause",
        "succeeded",
        "暂停采集计划",
        json!({"planId": plan_id}),
        None,
    )
    .await?;
    let plan = plan_rows(&state)
        .await?
        .into_iter()
        .find(|plan| plan["id"] == plan_id)
        .ok_or_else(|| ApiError::not_found(format!("采集计划 {plan_id}")))?;
    Ok(util::ok(&plan))
}

async fn resume_plan(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Path(plan_id): Path<String>,
    headers: HeaderMap,
    Json(_body): Json<Value>,
) -> Result<Response, ApiError> {
    let plan = plan_row_owned(&state, &plan_id, project.id()).await?;
    util::if_match(&headers, plan["etag"].as_str().unwrap_or(""))?;
    config::toggle(
        &state.config_dir,
        &config::LoadOptions::from_process(),
        "plan",
        &plan_id,
        true,
    )?;
    state.services.config_reload()?;
    crate::observe::append_audit(
        &state,
        "collection.plan.resume",
        "succeeded",
        "恢复采集计划",
        json!({"planId": plan_id}),
        None,
    )
    .await?;
    let plan = plan_rows(&state)
        .await?
        .into_iter()
        .find(|plan| plan["id"] == plan_id)
        .ok_or_else(|| ApiError::not_found(format!("采集计划 {plan_id}")))?;
    Ok(util::ok(&plan))
}

async fn list_runs(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let filter = format!(
        "runs:{}:{}:{}:{}",
        query.status.clone().unwrap_or_default(),
        query.source_id.clone().unwrap_or_default(),
        query.plan_id.clone().unwrap_or_default(),
        project.id()
    );
    let offset = util::page_offset(query.offset.as_deref(), query.cursor.as_deref(), &filter)?;
    let task_filter = TaskFilter {
        // 任务表按 `source/<slug>` 存归属，接口对外统一用裸 slug。
        source: query
            .source_id
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(|value| {
                if value.starts_with("source/") {
                    value.to_string()
                } else {
                    format!("source/{value}")
                }
            }),
        ..TaskFilter::default()
    };
    let result = state
        .services
        .queue_list(&task_filter, None, 200)
        .await
        .map_err(ApiError::from)?;
    let mut runs: Vec<Value> = result.rows.iter().map(run_view).collect();
    // 项目隔离：运行按其信源归属过滤。
    let allowed = visible_sources_opt(&state, &project, query.scope.as_deref()).await?;
    if let Some(allowed) = &allowed {
        runs.retain(|run| {
            let id = run["sourceId"].as_str().unwrap_or_default();
            !id.is_empty() && allowed.contains(id)
        });
    }
    // 历史任务把计划归属写成了信源 slug（旧版一信源一计划）；按当前计划的目标回填一等计划，
    // 否则计划维度的采集记录永远是空的。
    let owner = plan_owner_map(&state).await?;
    for run in &mut runs {
        attribute_plan(run, &owner);
    }
    if let Some(status) = query.status.as_deref().filter(|value| !value.is_empty()) {
        runs.retain(|run| run["status"] == status);
    }
    if let Some(plan) = query.plan_id.as_deref().filter(|value| !value.is_empty()) {
        let plan = plan.strip_prefix("plan/").unwrap_or(plan);
        runs.retain(|run| run["planId"].as_str() == Some(plan));
    }
    let total = runs.len();
    let items: Vec<Value> = runs.into_iter().skip(offset).take(limit + 1).collect();
    let more = items.len() > limit;
    let items: Vec<Value> = items.into_iter().take(limit).collect();
    let next = if more || offset + items.len() < total {
        Some(cursor(&filter, offset + limit))
    } else {
        None
    };
    Ok(util::ok(&page_total(items, next, total)))
}

fn run_view(row: &strata_client::TaskRow) -> Value {
    // 接口对外统一裸 slug（与信源/计划清单一致）；任务表内部带 `source/`、`plan/` 前缀。
    let source_id = row
        .source_id
        .strip_prefix("source/")
        .unwrap_or(&row.source_id)
        .to_string();
    let plan_id = row
        .plan_id
        .as_deref()
        .map(|id| id.strip_prefix("plan/").unwrap_or(id).to_string());
    json!({
        "id": format!("run_{}", row.task_id),
        "sourceId": source_id,
        // 计划归属来自任务 payload：信源与计划是采集记录仅有的两条上游线索。
        "planId": plan_id,
        "mode": if row.reason == "manual" { "manual" } else { "scheduled" },
        "status": match row.status.as_str() {
            "done" => "succeeded",
            "leased" => "running",
            "cancelled" => "cancelled",
            other => other,
        },
        "taskId": row.task_id,
        "queue": row.queue,
        "attempt": row.attempt,
        "locator": row.locator,
        "createdAt": row.created_at,
        "updatedAt": row.lease_until.clone().unwrap_or_else(|| row.created_at.clone()),
        "error": row.last_error,
    })
}

async fn get_run(
    State(state): State<Shared>,
    Path(run_id): Path<String>,
) -> Result<Response, ApiError> {
    let task_id = run_id.trim_start_matches("run_");
    let filter = TaskFilter {
        query: Some(task_id.to_string()),
        ..TaskFilter::default()
    };
    let page = state
        .services
        .queue_list(&filter, None, 5)
        .await
        .map_err(ApiError::from)?;
    let row = page
        .rows
        .iter()
        .find(|row| row.task_id == task_id)
        .ok_or_else(|| ApiError::not_found(format!("采集运行 {run_id}")))?;
    let mut view = run_view(row);
    let owner = plan_owner_map(&state).await?;
    attribute_plan(&mut view, &owner);
    Ok(util::ok(&view))
}

/// 信源 slug → 计划 id：按当前计划的目标反查，只给历史遗留归属用。
async fn plan_owner_map(
    state: &AppState,
) -> Result<std::collections::BTreeMap<String, String>, ApiError> {
    let mut owner = std::collections::BTreeMap::new();
    for plan in plan_rows(state).await? {
        let Some(id) = plan["id"].as_str() else {
            continue;
        };
        for target in plan["targets"].as_array().cloned().unwrap_or_default() {
            if let Some(value) = target.as_str() {
                let bare = value.strip_prefix("source/").unwrap_or(value);
                owner
                    .entry(bare.to_string())
                    .or_insert_with(|| id.to_string());
            }
        }
    }
    Ok(owner)
}

/// 归属为空或等于信源 slug 时，按目标反查补成计划 id；已经是计划 id 的不动。
fn attribute_plan(
    run: &mut Value,
    owner: &std::collections::BTreeMap<String, String>,
) {
    let source = run["sourceId"].as_str().unwrap_or_default().to_string();
    let plan = run["planId"].as_str().unwrap_or_default().to_string();
    if !(plan.is_empty() || plan == source) {
        return;
    }
    if let Some(id) = owner.get(&source) {
        run["planId"] = json!(id);
    }
}

async fn create_run(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["sourceId", "mode", "locator", "reason"])?;
    let source_id = util::non_empty(map, "sourceId")?;
    let mode = util::text(map, "mode")?;
    if !matches!(mode.as_str(), "normal" | "dryRun" | "singleLocator") {
        return Err(ApiError::validation(
            "mode",
            "enum",
            "只支持 normal、dryRun 或 singleLocator",
        ));
    }
    let locator = util::optional_text(map, "locator")?;
    if mode == "singleLocator" && locator.is_none() {
        return Err(ApiError::validation(
            "locator",
            "required",
            "单选地址模式需要 locator",
        ));
    }
    let reason = util::optional_text(map, "reason")?;
    let run_id = crate::new_id("run");
    let operation = crate::operations::start(
        &state,
        "collection.run",
        vec![json!({"kind": "collectionRun", "id": run_id})],
        json!({
            "kind": "collection.run",
            "runId": run_id,
            "sourceId": source_id,
            "mode": mode,
            "locator": locator,
            "reason": reason,
        }),
    )
    .await?;
    Ok(accepted(&operation))
}

pub async fn execute_run(
    state: &AppState,
    operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let source_id = action["sourceId"].as_str().unwrap_or_default();
    let mode = action["mode"].as_str().unwrap_or("normal");
    let locator = action["locator"].as_str().map(str::to_string);
    if crate::operations::check_cancel(state, operation_id)
        .await
        .is_err()
    {
        return Err(ApiError::conflict("操作已取消"));
    }
    let row = state
        .services
        .task_create(
            &format!("source/{source_id}"),
            Some(QueueName::Free),
            true,
            locator,
        )
        .await
        .map_err(ApiError::from)?;
    let task_id = row.task_id.clone();
    crate::observe::append_event(
        state,
        "collection.run",
        "采集运行已创建",
        json!({"runId": action["runId"], "sourceId": source_id, "mode": mode, "taskId": task_id, "status": row.status}),
        crate::observe::Refs {
            operation_id: Some(operation_id.to_string()),
            source_id: Some(source_id.to_string()),
            task_id: Some(task_id.clone()),
            ..crate::observe::Refs::default()
        },
    )
    .await?;
    Ok(json!({
        "partial": false,
        "runId": action["runId"],
        "sourceId": source_id,
        "mode": mode,
        "taskId": task_id,
        "status": row.status,
    }))
}
