//! 持久化 Operation、任务、队列与健康观测。

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Value, json};
use strata_client::{TaskFilter, TaskRow};

use crate::error::ApiError;
use crate::util::{self, cursor, cursor_offset, page};
use crate::{AppState, Shared, new_id, now};

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/operations/tasks", get(list_tasks))
        .route("/operations/tasks/batch", post(batch_tasks))
        .route("/operations/tasks/purge", post(purge_tasks))
        .route("/operations/queues", get(list_queues))
        .route("/operations/health", get(current_health))
        .route("/operations/healthHistory", get(list_health_history))
        .route("/operations/healthRecomputations", post(recompute_health))
        .route("/operations/{operationId}", get(get_operation))
        .route("/operations/{operationId}/events", get(operation_events))
        .route("/operations/{operationId}/cancel", post(cancel_operation))
        .route("/operations/{operationId}/retry", post(retry_operation))
        .route("/operations", get(list_operations))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListQuery {
    #[serde(default)]
    limit: Option<String>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    source_id: Option<String>,
}

/// 异步动作的统一入口：先落 Operation，再交给后台 worker。
pub async fn start(
    state: &Arc<AppState>,
    kind: &str,
    resource_refs: Vec<Value>,
    action: Value,
) -> Result<Value, ApiError> {
    let operation = json!({
        "operationId": new_id("op"),
        "kind": kind,
        "status": "queued",
        "progress": 0.0,
        "cancellable": true,
        "retryable": false,
        "retryOf": Value::Null,
        "resourceRefs": resource_refs,
        "resultSummary": Value::Null,
        "errorSummary": Value::Null,
        "createdAt": now(),
        "completedAt": Value::Null,
    });
    let id = operation["operationId"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    state
        .store
        .insert_operation(&operation, &action, &crate::error::current_request_id())
        .await?;
    state
        .store
        .append_operation_event(&id, "queued", "操作已接受", None)
        .await?;
    let state = Arc::clone(state);
    tokio::spawn(async move {
        run(state, id, action).await;
    });
    Ok(operation)
}

pub fn accepted(operation: &Value) -> Response {
    let id = operation["operationId"].as_str().unwrap_or_default();
    let mut response = (StatusCode::ACCEPTED, Json(operation.clone())).into_response();
    if let Ok(location) = format!("/api/v1/operations/{id}").parse() {
        response.headers_mut().insert(header::LOCATION, location);
    }
    response
}

async fn run(state: Shared, id: String, action: Value) {
    if cancelled(&state, &id).await {
        finish(&state, &id, "cancelled", 0.0, None, None, false).await;
        return;
    }
    update(&state, &id, "running", 0.1).await;
    let kind = action
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let outcome = match kind.as_str() {
        "source.delete" => crate::catalog::execute_source_delete(&state, &id, &action).await,
        "source.probe" => crate::catalog::execute_source_probe(&state, &id, &action).await,
        "source.analysis" => crate::catalog::execute_source_analysis(&state, &id, &action).await,
        "binding.rebind" => crate::bindings::execute_binding_rebind(&state, &id, &action).await,
        "adapter.draft" => crate::catalog::execute_adapter_draft(&state, &id, &action).await,
        "adapter.approve" => crate::catalog::execute_adapter_approve(&state, &id, &action).await,
        "collection.run" => crate::collection::execute_run(&state, &id, &action).await,
        "content.batch" => crate::content::execute_batch(&state, &id, &action).await,
        "content.replay" => crate::content::execute_replay(&state, &id, &action).await,
        "production.run" => crate::production::execute_run(&state, &id, &action).await,
        "production.retry" => crate::production::execute_retry(&state, &id, &action).await,
        "publication.publish" => crate::publication::execute_publish(&state, &id, &action).await,
        "publication.schedule" => {
            crate::publication::execute_scheduled_publish(&state, &id, &action).await
        }
        "delivery.retry" => crate::publication::execute_delivery_retry(&state, &id, &action).await,
        "notification.test" => {
            crate::publication::execute_notification_test(&state, &id, &action).await
        }
        "egressHost.reset" => crate::publication::execute_egress_reset(&state, &id, &action).await,
        "settings.apply" => crate::settings::execute_apply(&state, &id, &action).await,
        "settings.rollback" => crate::settings::execute_rollback(&state, &id, &action).await,
        "health.recompute" => execute_health(&state, &id, &action).await,
        "logs.export" => crate::observe::execute_export(&state, &id, &action).await,
        "tasks.batch" => execute_task_batch(&state, &id, &action).await,
        "maintenance.reset" => crate::maintenance::execute_reset(&state, &id, &action).await,
        "maintenance.factoryReset" => {
            crate::maintenance::execute_factory_reset(&state, &id, &action).await
        }
        other => Err(ApiError::internal(format!("未知操作类型: {other}"))),
    };
    if cancelled(&state, &id).await {
        finish(&state, &id, "cancelled", 1.0, None, None, false).await;
        return;
    }
    match outcome {
        Ok(result) => {
            let partial = result
                .get("partial")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let status = if partial {
                "partiallySucceeded"
            } else {
                "succeeded"
            };
            finish(&state, &id, status, 1.0, Some(result), None, partial).await;
        }
        Err(error) => {
            let retryable = error.retryable;
            let summary = error.body()["error"].clone();
            finish(&state, &id, "failed", 1.0, None, Some(summary), retryable).await;
        }
    }
}

async fn update(state: &AppState, id: &str, status: &str, progress: f64) {
    let _ = state
        .store
        .update_operation(
            id,
            &json!({
                "status": status,
                "progress": progress,
                "cancellable": matches!(status, "queued" | "running"),
            }),
        )
        .await;
}

async fn finish(
    state: &AppState,
    id: &str,
    status: &str,
    progress: f64,
    result: Option<Value>,
    error: Option<Value>,
    retryable: bool,
) {
    let patch = json!({
        "status": status,
        "progress": progress,
        "cancellable": false,
        "retryable": retryable || matches!(status, "failed" | "cancelled" | "partiallySucceeded"),
        "resultSummary": result,
        "errorSummary": error,
        "completedAt": now(),
    });
    let _ = state.store.update_operation(id, &patch).await;
    let _ = state
        .store
        .append_operation_event(id, status, "操作结束", result.as_ref())
        .await;
    let level = if status == "failed" { "error" } else { "info" };
    let _ = crate::observe::append_log(
        state,
        level,
        "operation",
        &format!("操作 {status}"),
        json!({"status": status, "result": result, "error": error}),
        Some(id),
    )
    .await;
}

pub async fn cancelled(state: &AppState, id: &str) -> bool {
    state
        .store
        .get_operation(id)
        .await
        .ok()
        .flatten()
        .and_then(|stored| stored.operation["status"].as_str().map(str::to_string))
        .is_some_and(|status| status == "cancelling" || status == "cancelled")
}

pub async fn check_cancel(state: &AppState, id: &str) -> Result<(), ApiError> {
    if cancelled(state, id).await {
        Err(ApiError::conflict("操作已取消"))
    } else {
        Ok(())
    }
}

/// 进程重启后恢复未完成的操作：清空/恢复出厂与排期发布继续执行，
/// 其余无法安全重放的操作标记失败并保留手动重试入口。
pub async fn recover_interrupted(state: std::sync::Arc<AppState>) {
    let Ok(operations) = state.store.list_operations(None, 500).await else {
        return;
    };
    for operation in operations {
        let status = operation["status"].as_str().unwrap_or_default();
        if !matches!(status, "queued" | "running" | "cancelling") {
            continue;
        }
        let Some(id) = operation["operationId"].as_str().map(str::to_string) else {
            continue;
        };
        let Ok(Some(stored)) = state.store.get_operation(&id).await else {
            continue;
        };
        let kind = stored.action["kind"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if matches!(
            kind.as_str(),
            "maintenance.reset" | "maintenance.factoryReset"
        ) {
            let _ = state.resetting.compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            );
            let task_state = std::sync::Arc::clone(&state);
            tokio::spawn(async move {
                run(task_state, id, stored.action).await;
            });
            continue;
        }
        if kind == "publication.schedule" {
            let publication_id = stored.action["publicationId"].as_str().unwrap_or_default();
            if let Ok(Some(doc)) = state
                .store
                .get_document("publication", publication_id)
                .await
                && doc.body["status"] == "scheduled"
            {
                let task_state = std::sync::Arc::clone(&state);
                tokio::spawn(async move {
                    run(task_state, id, stored.action).await;
                });
                continue;
            }
        }
        crate::publication::recover_interrupted(&state, &stored.action).await;
        crate::production::recover_interrupted(&state, &stored.action).await;
        let failed = status != "cancelling";
        let error = failed.then(|| {
            json!({
                "code": "interrupted",
                "message": "进程重启，操作已中断，可手动重试",
                "retryable": true,
            })
        });
        finish(
            &state,
            &id,
            if failed { "failed" } else { "cancelled" },
            1.0,
            None,
            error,
            true,
        )
        .await;
    }
}

pub(crate) async fn get_operation(
    State(state): State<Shared>,
    Path(operation_id): Path<String>,
) -> Result<Response, ApiError> {
    let stored = state
        .store
        .get_operation(&operation_id)
        .await?
        .ok_or_else(|| ApiError::not_found(&operation_id))?;
    Ok(util::ok(&stored.operation))
}

pub(crate) async fn list_operations(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let filter = format!("operations:{}", query.status.clone().unwrap_or_default());
    let offset = cursor_offset(query.cursor.as_deref(), &filter)?;
    let status = query
        .status
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let mut items = state
        .store
        .list_operations(status, limit + offset + 1)
        .await?;
    items.drain(..offset.min(items.len()));
    let next = if items.len() > limit {
        Some(cursor(&filter, offset + limit))
    } else {
        None
    };
    items.truncate(limit);
    Ok(util::ok(&page(items, next)))
}

pub(crate) async fn operation_events(
    State(state): State<Shared>,
    Path(operation_id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let after = query
        .cursor
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("0")
        .parse::<i64>()
        .map_err(|_| ApiError::cursor_expired())?;
    let mut items = state
        .store
        .list_operation_events(&operation_id, after, limit)
        .await?;
    let next = if items.len() > limit {
        items
            .get(limit - 1)
            .and_then(|item| item["cursor"].as_str())
            .map(str::to_string)
    } else {
        None
    };
    items.truncate(limit);
    Ok(util::ok(&page(items, next)))
}

pub(crate) async fn cancel_operation(
    State(state): State<Shared>,
    Path(operation_id): Path<String>,
    _headers: HeaderMap,
) -> Result<Response, ApiError> {
    let stored = state
        .store
        .get_operation(&operation_id)
        .await?
        .ok_or_else(|| ApiError::not_found(&operation_id))?;
    let status = stored.operation["status"].as_str().unwrap_or("failed");
    if matches!(
        status,
        "succeeded" | "partiallySucceeded" | "failed" | "cancelled"
    ) {
        return Err(ApiError::not_cancellable());
    }
    if !stored.operation["cancellable"].as_bool().unwrap_or(false) {
        return Err(ApiError::not_cancellable());
    }
    state
        .store
        .update_operation(
            &operation_id,
            &json!({"status": "cancelling", "cancellable": false}),
        )
        .await?;
    state
        .store
        .append_operation_event(&operation_id, "cancel", "用户请求取消", None)
        .await?;
    let stored = state
        .store
        .get_operation(&operation_id)
        .await?
        .ok_or_else(|| ApiError::not_found(&operation_id))?;
    Ok(accepted(&stored.operation))
}

pub(crate) async fn retry_operation(
    State(state): State<Shared>,
    Path(operation_id): Path<String>,
    Json(_body): Json<Value>,
) -> Result<Response, ApiError> {
    let stored = state
        .store
        .get_operation(&operation_id)
        .await?
        .ok_or_else(|| ApiError::not_found(&operation_id))?;
    let retryable = stored.operation["retryable"].as_bool().unwrap_or(false);
    if !retryable {
        return Err(ApiError::not_retryable());
    }
    let refs = stored.operation["resourceRefs"].clone();
    let kind = stored.operation["kind"]
        .as_str()
        .unwrap_or("operation.retry")
        .to_string();
    let mut action = stored.action.clone();
    if let Value::Object(map) = &mut action {
        map.insert("retryOf".into(), Value::String(operation_id.clone()));
    }
    let mut operation = start(
        &state,
        &kind,
        refs.as_array().cloned().unwrap_or_default(),
        action,
    )
    .await?;
    if let Some(map) = operation.as_object_mut() {
        map.insert("retryOf".into(), Value::String(operation_id.clone()));
    }
    let id = operation["operationId"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    state
        .store
        .update_operation(&id, &json!({"retryOf": operation_id.clone()}))
        .await?;
    Ok(accepted(&operation))
}

async fn execute_health(
    state: &Arc<AppState>,
    _id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let source_ids: Vec<String> = action
        .get("sourceIds")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    state.services.health_evaluate().await?;
    let history = health_snapshot(state, &source_ids).await?;
    state
        .store
        .insert_document("health_history", &new_id("h"), &history)
        .await?;
    Ok(
        json!({"partial": false, "sources": history["sources"], "evaluatedAt": history["evaluatedAt"]}),
    )
}

async fn health_snapshot(state: &AppState, source_filter: &[String]) -> Result<Value, ApiError> {
    let statuses = state
        .services
        .planner_statuses()
        .await
        .map_err(ApiError::from)?;
    let mut sources = Vec::new();
    for (source, status) in statuses {
        if !source_filter.is_empty() && !source_filter.contains(&source) {
            continue;
        }
        sources.push(json!({
            "sourceId": source,
            "status": status.health_status,
            "reason": status.last_error,
            "lastOkAt": status.last_ok_at,
            "todayYield": status.today_yield,
            "alert": status.alert,
        }));
    }
    let queues = state
        .services
        .queue_states()
        .await
        .map_err(ApiError::from)?;
    let counts = state
        .services
        .store_counts()
        .await
        .map_err(ApiError::from)?;
    let egress = state
        .services
        .egress_report()
        .await
        .map_err(ApiError::from)?;
    Ok(json!({
        "evaluatedAt": now(),
        "status": if sources.iter().any(|item| item["status"] == "failed") { "degraded" } else { "online" },
        "sources": sources,
        "queues": serde_json::to_value(queues).unwrap_or(Value::Null),
        "storage": serde_json::to_value(counts).unwrap_or(Value::Null),
        "egress": serde_json::to_value(egress).unwrap_or(Value::Null),
    }))
}

async fn current_health(State(state): State<Shared>) -> Result<Response, ApiError> {
    let snapshot = health_snapshot(&state, &[]).await?;
    Ok(util::ok(&snapshot))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryQuery {
    #[serde(default)]
    limit: Option<String>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    source_id: Option<String>,
}

async fn list_health_history(
    State(state): State<Shared>,
    Query(query): Query<HistoryQuery>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let filter = format!("health:{}", query.source_id.clone().unwrap_or_default());
    let offset = cursor_offset(query.cursor.as_deref(), &filter)?;
    let docs = state.store.list_documents("health_history").await?;
    let retention = crate::settings::value(&state, "retention.healthHistoryLimit")
        .await?
        .as_u64()
        .unwrap_or(200)
        .clamp(10, 5000) as usize;
    let mut docs = docs;
    docs.sort_by(|left, right| {
        right.body["evaluatedAt"]
            .as_str()
            .unwrap_or("")
            .cmp(left.body["evaluatedAt"].as_str().unwrap_or(""))
    });
    docs.truncate(retention);
    let total = docs.len();
    let mut items: Vec<Value> = docs
        .into_iter()
        .skip(offset)
        .take(limit + 1)
        .map(|doc| doc.body)
        .collect();
    if let Some(source) = query.source_id.as_deref().filter(|value| !value.is_empty()) {
        for item in &mut items {
            if let Some(list) = item.get_mut("sources").and_then(Value::as_array_mut) {
                list.retain(|entry| entry["sourceId"] == source);
            }
        }
    }
    let more = items.len() > limit;
    let items: Vec<Value> = items.into_iter().take(limit).collect();
    let next = if more || offset + items.len() < total {
        Some(cursor(&filter, offset + limit))
    } else {
        None
    };
    Ok(util::ok(&page(items, next)))
}

async fn recompute_health(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["sourceIds"])?;
    let source_ids = util::optional_strings(map, "sourceIds")?.unwrap_or_default();
    let operation = start(
        &state,
        "health.recompute",
        vec![json!({"kind": "health", "id": "current"})],
        json!({"kind": "health.recompute", "sourceIds": source_ids}),
    )
    .await?;
    Ok(accepted(&operation))
}

async fn list_tasks(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let status = query
        .status
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let kind = query
        .kind
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let filter = format!(
        "tasks:{}:{}:{}",
        status.unwrap_or(""),
        query.source_id.clone().unwrap_or_default(),
        kind.unwrap_or("")
    );
    let offset = cursor_offset(query.cursor.as_deref(), &filter)?;
    let mut items = Vec::new();
    if kind.is_none() || kind == Some("collection") || kind == Some("derived") {
        let task_filter = TaskFilter {
            source: query.source_id.clone().filter(|value| !value.is_empty()),
            ..TaskFilter::default()
        };
        let result = state
            .services
            .queue_list(&task_filter, None, 200)
            .await
            .map_err(ApiError::from)?;
        for row in result.rows {
            let view = task_view(&row);
            if status.is_none_or(|expected| view["status"] == expected) {
                items.push(view);
            }
        }
    }
    if kind.is_none()
        || kind == Some("production")
        || kind == Some("publication")
        || kind == Some("maintenance")
    {
        for doc in state.store.list_documents("task").await? {
            if let Some(status) = status
                && doc.body["status"] != status
            {
                continue;
            }
            if let Some(want) = kind
                && doc.body["kind"] != want
            {
                continue;
            }
            items.push(doc.body);
        }
    }
    items.sort_by(|left, right| {
        right["createdAt"]
            .as_str()
            .unwrap_or("")
            .cmp(left["createdAt"].as_str().unwrap_or(""))
    });
    let total = items.len();
    let items: Vec<Value> = items.into_iter().skip(offset).take(limit + 1).collect();
    let more = items.len() > limit;
    let items: Vec<Value> = items.into_iter().take(limit).collect();
    let next = if more || offset + items.len() < total {
        Some(cursor(&filter, offset + limit))
    } else {
        None
    };
    Ok(util::ok(&page(items, next)))
}

fn task_view(row: &TaskRow) -> Value {
    let status = match row.status.as_str() {
        "done" => "succeeded",
        "leased" => "running",
        // 协作取消会写入固定文案；只有携带真实错误的取消才算失败。
        "cancelled"
            if row
                .last_error
                .as_deref()
                .is_some_and(|error| !error.trim().is_empty() && error != "已取消") =>
        {
            "failed"
        }
        other => other,
    };
    json!({
        "id": row.task_id,
        "kind": "collection",
        "status": status,
        "title": row.source_id,
        "sourceId": row.source_id,
        "queue": row.queue,
        "reason": row.reason,
        "attempt": row.attempt,
        "createdAt": row.created_at,
        "updatedAt": row.lease_until.clone().unwrap_or_else(|| row.created_at.clone()),
        "retryAt": row.retry_at,
        "error": row.last_error,
        "locator": row.locator,
    })
}

async fn batch_tasks(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["ids", "action", "tags"])?;
    let ids = util::strings(map, "ids")?;
    if ids.is_empty() || ids.len() > 1000 {
        return Err(ApiError::validation(
            "ids",
            "outOfRange",
            "数量需要在 1 到 1000 之间",
        ));
    }
    let action = util::text(map, "action")?;
    if !matches!(action.as_str(), "cancel" | "retry" | "requeue" | "delete") {
        return Err(ApiError::validation(
            "action",
            "enum",
            "任务只支持 cancel、retry、requeue 或 delete",
        ));
    }
    let operation = start(
        &state,
        "tasks.batch",
        vec![json!({"kind": "taskBatch", "id": new_id("batch")})],
        json!({"kind": "tasks.batch", "ids": ids, "action": action}),
    )
    .await?;
    Ok(accepted(&operation))
}

async fn execute_task_batch(
    state: &AppState,
    _operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let ids = action["ids"].as_array().cloned().unwrap_or_default();
    let decision = action["action"].as_str().unwrap_or("requeue");
    let mut succeeded = Vec::new();
    let mut failed = Vec::new();
    for id in ids {
        let Some(id) = id.as_str() else { continue };
        if let Some(doc) = state.store.get_document("task", id).await? {
            if decision == "delete" {
                state.store.delete_document("task", id).await?;
                succeeded.push(json!({"id": id, "status": "deleted"}));
                continue;
            }
            let next = match decision {
                "cancel" => "cancelled",
                _ => "queued",
            };
            let mut body = doc.body;
            if let Some(map) = body.as_object_mut() {
                map.insert("status".into(), Value::String(next.into()));
                map.insert("updatedAt".into(), Value::String(now()));
            }
            state
                .store
                .update_document("task", id, &body, Some(doc.version))
                .await?;
            succeeded.push(json!({"id": id, "status": next}));
            continue;
        }
        let Ok(task_id) = strata_model::TaskId::new(id) else {
            failed.push(json!({"id": id, "code": "notFound", "message": "任务不存在"}));
            continue;
        };
        if decision == "delete" {
            let _ = state.services.queue_cancel(&task_id).await;
            match state.services.queue_remove(&task_id).await {
                Ok(true) => succeeded.push(json!({"id": id, "status": "deleted"})),
                Ok(false) => failed.push(json!({
                    "id": id,
                    "code": "notFound",
                    "message": "任务不存在"
                })),
                Err(error) => failed.push(json!({
                    "id": id,
                    "code": "dependencyUnavailable",
                    "message": error.to_string()
                })),
            }
            continue;
        }
        let outcome = match decision {
            "cancel" => state.services.queue_cancel(&task_id).await,
            _ => state.services.queue_requeue(&task_id).await,
        };
        match outcome {
            Ok(true) => succeeded.push(json!({"id": id, "status": decision})),
            Ok(false) => failed.push(json!({
                "id": id,
                "code": "conflict",
                "message": "当前状态不允许该动作"
            })),
            Err(error) => failed.push(json!({
                "id": id,
                "code": "dependencyUnavailable",
                "message": error.to_string()
            })),
        }
    }
    Ok(json!({
        "partial": !failed.is_empty() && !succeeded.is_empty(),
        "succeeded": succeeded,
        "failed": failed,
    }))
}

/// 清空任务：可按状态、类型、来源与创建时间筛选；活动任务先取消再物理删除。
async fn purge_tasks(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["status", "kind", "sourceId", "before", "dryRun"])?;
    let status =
        util::optional_text(map, "status")?.filter(|value| !value.is_empty() && value != "all");
    if let Some(value) = status.as_deref()
        && !matches!(
            value,
            "queued" | "running" | "succeeded" | "failed" | "cancelled"
        )
    {
        return Err(ApiError::validation("status", "enum", "未知任务状态"));
    }
    let kind =
        util::optional_text(map, "kind")?.filter(|value| !value.is_empty() && value != "all");
    let source = util::optional_text(map, "sourceId")?.filter(|value| !value.trim().is_empty());
    let before = util::optional_text(map, "before")?.filter(|value| !value.trim().is_empty());
    let dry_run = matches!(map.get("dryRun"), Some(Value::Bool(true)));
    let storage_status = status.as_deref().map(|value| match value {
        "running" => "leased",
        "succeeded" => "done",
        "failed" | "cancelled" => "cancelled",
        other => other,
    });
    let filter = TaskFilter {
        status: storage_status.map(str::to_string),
        source: source.clone(),
        before: before.clone(),
        ..TaskFilter::default()
    };
    let matched_store = state
        .services
        .queue_count(&filter)
        .await
        .map_err(ApiError::from)?;
    let control = control_tasks(
        &state,
        status.as_deref(),
        kind.as_deref(),
        source.as_deref(),
        before.as_deref(),
    )
    .await?;
    if dry_run {
        return Ok(util::ok(&json!({
            "matched": matched_store + control.len() as u64,
            "deleted": 0,
            "dryRun": true,
        })));
    }
    // 活动任务先协作取消，再由存储层物理删除。
    for active in ["queued", "leased"] {
        if storage_status.is_some() && storage_status != Some(active) {
            continue;
        }
        let active_filter = TaskFilter {
            status: Some(active.to_string()),
            source: source.clone(),
            before: before.clone(),
            ..TaskFilter::default()
        };
        let _ = cancel_matching(&state, &active_filter).await?;
    }
    let purge_filter = TaskFilter {
        status: storage_status.map(|value| {
            if matches!(value, "queued" | "leased") {
                "cancelled".to_string()
            } else {
                value.to_string()
            }
        }),
        source: source.clone(),
        before: before.clone(),
        ..TaskFilter::default()
    };
    let deleted_store = state
        .services
        .queue_purge(&purge_filter)
        .await
        .map_err(ApiError::from)?;
    let mut deleted_control = 0u64;
    for doc in &control {
        if state.store.delete_document("task", &doc.id).await? {
            deleted_control += 1;
        }
    }
    crate::observe::append_audit(
        &state,
        "tasks.purge",
        "succeeded",
        "清空任务",
        json!({
            "status": status,
            "kind": kind,
            "sourceId": source,
            "before": before,
            "deleted": deleted_store + deleted_control,
        }),
        None,
    )
    .await?;
    Ok(util::ok(&json!({
        "matched": matched_store + control.len() as u64,
        "deleted": deleted_store + deleted_control,
        "dryRun": false,
    })))
}

async fn cancel_matching(state: &AppState, filter: &TaskFilter) -> Result<u64, ApiError> {
    let mut cancelled = 0u64;
    let mut cursor: Option<String> = None;
    loop {
        let page = state
            .services
            .queue_list(filter, cursor.as_deref(), 500)
            .await
            .map_err(ApiError::from)?;
        for row in &page.rows {
            if let Ok(task_id) = strata_model::TaskId::new(&row.task_id)
                && state.services.queue_cancel(&task_id).await.unwrap_or(false)
            {
                cancelled += 1;
            }
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    Ok(cancelled)
}

async fn control_tasks(
    state: &AppState,
    status: Option<&str>,
    kind: Option<&str>,
    source: Option<&str>,
    before: Option<&str>,
) -> Result<Vec<crate::store::Document>, ApiError> {
    let mut out = Vec::new();
    for doc in state.store.list_documents("task").await? {
        if let Some(status) = status
            && doc.body["status"] != status
        {
            continue;
        }
        if let Some(kind) = kind
            && doc.body["kind"] != kind
        {
            continue;
        }
        if let Some(source) = source
            && doc.body["sourceId"] != source
        {
            continue;
        }
        if let Some(before) = before
            && doc.body["createdAt"].as_str().unwrap_or("") >= before
        {
            continue;
        }
        out.push(doc);
    }
    Ok(out)
}

async fn list_queues(State(state): State<Shared>) -> Result<Response, ApiError> {
    let states = state
        .services
        .queue_states()
        .await
        .map_err(ApiError::from)?;
    let items: Vec<Value> = states
        .into_iter()
        .map(|queue| {
            json!({
                "id": queue.queue,
                "queued": queue.queued,
                "running": queue.leased,
                "done": queue.done,
                "cancelled": queue.cancelled,
                "capacity": queue.capacity,
                "paused": queue.paused,
            })
        })
        .collect();
    Ok(util::ok(&page(items, None)))
}

/// 把控制面任务写入统一任务流。
pub async fn record_task(state: &AppState, task: Value) -> Result<(), ApiError> {
    let id = task["id"].as_str().unwrap_or_default().to_string();
    if id.is_empty() {
        return Ok(());
    }
    if let Some(current) = state.store.get_document("task", &id).await? {
        let mut body = current.body;
        if let (Value::Object(target), Value::Object(source)) = (&mut body, &task) {
            for (key, value) in source {
                target.insert(key.clone(), value.clone());
            }
        }
        state
            .store
            .update_document("task", &id, &body, Some(current.version))
            .await?;
    } else {
        state.store.insert_document("task", &id, &task).await?;
    }
    Ok(())
}
