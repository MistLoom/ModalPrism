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
use crate::util::{self, cursor, cursor_offset, page};
use crate::{AppState, Shared};

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/collection/plans", get(list_plans))
        .route(
            "/collection/plans/{planId}",
            get(get_plan).patch(update_plan),
        )
        .route("/collection/plans/{planId}/pause", post(pause_plan))
        .route("/collection/plans/{planId}/resume", post(resume_plan))
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
    status: Option<String>,
    #[serde(default)]
    source_id: Option<String>,
}

pub(crate) async fn plan_rows(state: &AppState) -> Result<Vec<Value>, ApiError> {
    let sources = crate::catalog::source_rows(state).await?;
    let mut plans = Vec::new();
    for source in sources {
        let id = source["id"].as_str().unwrap_or_default().to_string();
        let paused = state
            .store
            .get_document("plan_state", &id)
            .await?
            .map(|doc| doc.body["paused"].as_bool().unwrap_or(false))
            .unwrap_or(false);
        plans.push(json!({
            "id": id,
            "sourceId": id,
            "name": source["name"],
            "enabled": source["enabled"],
            "status": if paused { "paused" } else if source["enabled"].as_bool().unwrap_or(false) { "active" } else { "disabled" },
            "schedule": source["schedule"],
            "health": source["health"],
            "nextRunAt": source["nextRunAt"],
            "lastRunAt": source["lastRunAt"],
            "todayYield": source["todayYield"],
            "etag": source["etag"],
        }));
    }
    Ok(plans)
}

async fn list_plans(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let filter = format!("plans:{}", query.status.clone().unwrap_or_default());
    let offset = cursor_offset(query.cursor.as_deref(), &filter)?;
    let mut plans = plan_rows(&state).await?;
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
    Ok(util::ok(&page(items, next)))
}

async fn get_plan(
    State(state): State<Shared>,
    Path(plan_id): Path<String>,
) -> Result<Response, ApiError> {
    let plan = plan_rows(&state)
        .await?
        .into_iter()
        .find(|plan| plan["id"] == plan_id)
        .ok_or_else(|| ApiError::not_found(format!("采集计划 {plan_id}")))?;
    let mut response = util::ok(&plan);
    if let Ok(value) = plan["etag"].as_str().unwrap_or("").parse() {
        response
            .headers_mut()
            .insert(axum::http::header::ETAG, value);
    }
    Ok(response)
}

async fn update_plan(
    State(state): State<Shared>,
    Path(plan_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["schedule"])?;
    let schedule = util::optional_object(map, "schedule")?
        .ok_or_else(|| ApiError::validation("schedule", "required", "不能为空"))?;
    util::unknown(&schedule, &["mode", "intervalMs", "minimumIntervalMs"])?;
    let mode = util::text(&schedule, "mode")?;
    let interval_ms = schedule.get("intervalMs").and_then(Value::as_i64);
    let plan = plan_rows(&state)
        .await?
        .into_iter()
        .find(|plan| plan["id"] == plan_id)
        .ok_or_else(|| ApiError::not_found(format!("采集计划 {plan_id}")))?;
    util::if_match(&headers, plan["etag"].as_str().unwrap_or(""))?;
    let entry = config::entries(&state.config_dir, "source")?
        .into_iter()
        .find(|entry| entry.slug == plan_id)
        .ok_or_else(|| ApiError::not_found(format!("信源 {plan_id}")))?;
    let mut doc = entry.doc.as_object().cloned().unwrap_or_default();
    if mode == "push" {
        doc.remove("schedule");
    } else {
        let default_interval = crate::settings::value(&state, "collection.defaultIntervalMs")
            .await?
            .as_i64()
            .unwrap_or(1_800_000);
        let minimum = crate::settings::value(&state, "collection.minimumIntervalMs")
            .await?
            .as_i64()
            .unwrap_or(60_000)
            .max(1_000);
        let requested_floor = schedule
            .get("minimumIntervalMs")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let floor = minimum.max(requested_floor);
        let interval = interval_ms.unwrap_or(default_interval);
        if interval < floor {
            return Err(ApiError::validation(
                "schedule.intervalMs",
                "outOfRange",
                format!("间隔不能低于系统最小间隔 {floor}ms"),
            ));
        }
        doc.insert(
            "schedule".into(),
            json!({"interval": util::duration_text(interval)}),
        );
    }
    config::update(
        &state.config_dir,
        &config::LoadOptions::from_process(),
        &config::Draft {
            kind: "source".into(),
            slug: plan_id.clone(),
            doc: Value::Object(doc),
        },
    )?;
    state.services.config_reload()?;
    crate::observe::append_audit(
        &state,
        "collection.plan.update",
        "succeeded",
        "调整采集计划",
        json!({"planId": plan_id, "mode": mode, "intervalMs": interval_ms}),
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
    Path(plan_id): Path<String>,
    headers: HeaderMap,
    Json(_body): Json<Value>,
) -> Result<Response, ApiError> {
    let plan = plan_rows(&state)
        .await?
        .into_iter()
        .find(|plan| plan["id"] == plan_id)
        .ok_or_else(|| ApiError::not_found(format!("采集计划 {plan_id}")))?;
    util::if_match(&headers, plan["etag"].as_str().unwrap_or(""))?;
    config::toggle(
        &state.config_dir,
        &config::LoadOptions::from_process(),
        "source",
        &plan_id,
        false,
    )?;
    upsert_plan_state(&state, &plan_id, true).await?;
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
    Path(plan_id): Path<String>,
    headers: HeaderMap,
    Json(_body): Json<Value>,
) -> Result<Response, ApiError> {
    let plan = plan_rows(&state)
        .await?
        .into_iter()
        .find(|plan| plan["id"] == plan_id)
        .ok_or_else(|| ApiError::not_found(format!("采集计划 {plan_id}")))?;
    util::if_match(&headers, plan["etag"].as_str().unwrap_or(""))?;
    config::toggle(
        &state.config_dir,
        &config::LoadOptions::from_process(),
        "source",
        &plan_id,
        true,
    )?;
    upsert_plan_state(&state, &plan_id, false).await?;
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

async fn upsert_plan_state(state: &AppState, plan_id: &str, paused: bool) -> Result<(), ApiError> {
    let body = json!({"id": plan_id, "paused": paused, "updatedAt": crate::now()});
    match state.store.get_document("plan_state", plan_id).await? {
        Some(doc) => {
            state
                .store
                .update_document("plan_state", plan_id, &body, Some(doc.version))
                .await?;
        }
        None => {
            state
                .store
                .insert_document("plan_state", plan_id, &body)
                .await?;
        }
    }
    Ok(())
}

async fn list_runs(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let filter = format!(
        "runs:{}:{}",
        query.status.clone().unwrap_or_default(),
        query.source_id.clone().unwrap_or_default()
    );
    let offset = cursor_offset(query.cursor.as_deref(), &filter)?;
    let task_filter = TaskFilter {
        source: query.source_id.clone().filter(|value| !value.is_empty()),
        ..TaskFilter::default()
    };
    let result = state
        .services
        .queue_list(&task_filter, None, 200)
        .await
        .map_err(ApiError::from)?;
    let mut runs: Vec<Value> = result.rows.iter().map(run_view).collect();
    if let Some(status) = query.status.as_deref().filter(|value| !value.is_empty()) {
        runs.retain(|run| run["status"] == status);
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
    Ok(util::ok(&page(items, next)))
}

fn run_view(row: &strata_client::TaskRow) -> Value {
    json!({
        "id": format!("run_{}", row.task_id),
        "sourceId": row.source_id,
        "mode": if row.reason == "manual" { "manual" } else { "scheduled" },
        "status": match row.status.as_str() {
            "done" => "succeeded",
            "leased" => "running",
            "cancelled" => "cancelled",
            other => other,
        },
        "taskId": row.task_id,
        "queue": row.queue,
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
    Ok(util::ok(&run_view(row)))
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
