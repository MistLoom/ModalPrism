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
    })))
}

async fn overview(State(state): State<Shared>) -> Result<Response, ApiError> {
    let sources = crate::catalog::source_rows(&state).await?;
    let summary = state
        .services
        .strata()
        .summary()
        .await
        .map_err(ApiError::from)?;
    let stats = state.services.queue_stats().await.map_err(ApiError::from)?;
    let queues = state
        .services
        .queue_states()
        .await
        .map_err(ApiError::from)?;
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
    for row in &stats.by_status {
        if row.0 == "failed" {
            // 失败任务数由 stats 汇总，具体条目在任务页按状态过滤。
        }
    }
    let publications = state.store.list_documents("publication").await?;
    let failed_publications = publications
        .iter()
        .filter(|doc| doc.body["status"] == "failed")
        .count();
    for doc in publications
        .iter()
        .filter(|doc| doc.body["status"] == "failed")
        .take(5)
    {
        attention.push(json!({
            "kind": "publication",
            "id": doc.body["id"],
            "title": doc.body["title"],
            "status": "failed",
            "message": "发布存在失败渠道",
            "filter": {"status": "failed"},
        }));
    }
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
            "content": summary.total,
            "tombstoned": summary.tombstoned,
            "tasksQueued": stats.by_status.get("queued").copied().unwrap_or(0),
            "tasksRunning": stats.by_status.get("leased").copied().unwrap_or(0),
            "tasksFailed": stats.by_status.get("failed").copied().unwrap_or(0),
            "failedPublications": failed_publications,
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
        "contentKinds": summary.variants,
        "links": {
            "sources": "/#/sources",
            "tasks": "/#/tasks",
            "content": "/#/content",
            "logs": "/#/logs",
        },
    });
    Ok(util::ok(&overview))
}
