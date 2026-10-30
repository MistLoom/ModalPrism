//! 系统状态与概览。

use axum::Router;
use axum::extract::State;
use axum::response::Response;
use axum::routing::get;
use serde_json::json;

use crate::error::ApiError;
use crate::util;
use crate::{API_VERSION, Shared};

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/system/status", get(status))
        .route("/system/overview", get(overview))
}

async fn status(State(state): State<Shared>) -> Result<Response, ApiError> {
    let statuses = state
        .services
        .planner_statuses()
        .await
        .map_err(ApiError::from)?;
    let degraded = statuses
        .iter()
        .any(|(_, status)| status.health_status == "failed");
    let status = if state.in_maintenance() {
        "maintenance"
    } else if degraded {
        "degraded"
    } else {
        "online"
    };
    Ok(util::ok(&json!({
        "status": status,
        "version": API_VERSION,
        "displayName": crate::settings::value(&state, "general.displayName")
            .await?
            .as_str()
            .unwrap_or("ModalPrism"),
        "generation": state.services.config_generation(),
        "startedAt": state.started_at,
        // 宿主注入能力就绪度：rawprism 解析钩子未接入时，前端据此前置降级而非点击报错。
        "capabilities": {
            "parse": state.parse_hook.get().is_some(),
        },
    })))
}

async fn overview(
    State(state): State<Shared>,
    project: crate::ActiveProject,
) -> Result<Response, ApiError> {
    // 项目隔离：计数与列表都只算当前项目可见的信源及其内容。
    let active = project.id().to_string();
    let mut sources = crate::catalog::source_rows(&state).await?;
    sources.retain(|source| crate::catalog::source_visible(source, &active));
    let queues = state
        .services
        .queue_states()
        .await
        .map_err(ApiError::from)?;
    // 信源列表按“可见”口径（含共享），内容/任务等派生资源按“自有”口径：
    // 共享只放开信源声明，不把其他项目的采集产物与运行数据算进本项目。
    let mut owned: Vec<String> = crate::catalog::owned_source_ids(&state, &active)
        .await?
        .into_iter()
        .collect();
    owned.sort();
    let (content_total, content_tombstoned, content_kinds) = if owned.is_empty() {
        (0usize, 0usize, Vec::new())
    } else {
        let allowed = owned.as_slice();
        let total = crate::content::item_count(&state, None, None, None, Some(allowed)).await?;
        let tombstoned =
            crate::content::item_count(&state, None, None, Some("tombstoned"), Some(allowed)).await?;
        let kinds = crate::content::variant_counts(&state, Some(allowed)).await?;
        (total, tombstoned, kinds)
    };
    // 任务计数也按自有信源封口：全局 queue_stats 会把其他项目的排队/失败算进来。
    let queue_rows = state
        .services
        .queue_list(&strata_client::TaskFilter::default(), None, 1_000)
        .await
        .map_err(ApiError::from)?
        .rows;
    let mut tasks_queued = 0usize;
    let mut tasks_running = 0usize;
    let mut tasks_failed = 0usize;
    for row in &queue_rows {
        if !owned.iter().any(|id| id == &row.source_id) {
            continue;
        }
        match row.status.as_str() {
            "queued" => tasks_queued += 1,
            "leased" => tasks_running += 1,
            "failed" => tasks_failed += 1,
            _ => {}
        }
    }
    let mut healthy = 0usize;
    let mut attention = Vec::new();
    for source in &sources {
        let health = source["health"].as_str().unwrap_or("healthy");
        if health == "healthy" {
            healthy += 1;
        }
        if matches!(
            health,
            "degraded" | "failed" | "suspicious" | "configurationError"
        ) {
            attention.push(json!({
                "kind": "source",
                "id": source["id"],
                "title": source["name"],
                "status": health,
                "message": source["healthReason"],
                "filter": {"sourceId": source["id"]},
            }));
        }
    }
    // 投递失败：这里只给计数，按记录归属封口；旧记录无 project 时回落默认项目。
    let delivery_records = state.store.list_documents("delivery_record").await?;
    let failed_deliveries = delivery_records
        .iter()
        .filter(|doc| {
            doc.body["status"] == "failed"
                && doc.body["project"]
                    .as_str()
                    .unwrap_or(crate::projects::DEFAULT_PROJECT_ID)
                    == active
        })
        .count();
    let recent = state.store.list_operations(None, 8).await?;
    let overview = json!({
        "generatedAt": crate::now(),
        "status": if state.in_maintenance() { "maintenance" } else if attention.is_empty() { "online" } else { "degraded" },
        "displayName": crate::settings::value(&state, "general.displayName")
            .await?
            .as_str()
            .unwrap_or("ModalPrism"),
        "generation": state.services.config_generation(),
        "counts": {
            "sources": sources.len(),
            "enabledSources": sources.iter().filter(|source| source["enabled"].as_bool().unwrap_or(false)).count(),
            "healthySources": healthy,
            "attention": attention.len(),
            // 「待处理」以未读通知为准：与通知中心同一口径，点进去能看到同数条目。
            "unreadNotifications": crate::notifications::unread_count(&state).await,
            "content": content_total,
            "tombstoned": content_tombstoned,
            "tasksQueued": tasks_queued,
            "tasksRunning": tasks_running,
            "tasksFailed": tasks_failed,
            "failedDeliveries": failed_deliveries,
        },
        "queues": queues.iter().map(|queue| json!({
            "id": queue.queue,
            "queued": queue.queued,
            "running": queue.leased,
            "capacity": queue.capacity,
            "paused": queue.paused,
        })).collect::<Vec<_>>(),
        "attention": attention,
        "recentOperations": recent,
        "sources": sources.iter().take(8).map(|source| json!({
            "id": source["id"],
            "name": source["name"],
            "health": source["health"],
            "todayYield": source["todayYield"],
            "nextRunAt": source["nextRunAt"],
        })).collect::<Vec<_>>(),
        "contentKinds": content_kinds,
        "links": {
            "sources": "/#/sources",
            "tasks": "/#/tasks",
            "content": "/#/content",
            "logs": "/#/logs",
        },
    });
    Ok(util::ok(&overview))
}
