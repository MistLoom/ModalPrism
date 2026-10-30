//! 框架监测投影：把 LEAF `monitor` 的内存视图转成控制台可轮询的只读接口。
//!
//! 这里不参与探针实现：宿主（collector）启动时把 `MonitorApp` 包装成
//! [`crate::MonitorHook`] 注入，领域 API 只做投影与降级。

use axum::Router;
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::util;
use crate::{Shared, now};

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MonitorQuery {
    #[serde(default)]
    tail: Option<usize>,
}

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/monitor/chains", get(chains))
        .route("/monitor/events", get(events))
        .route("/monitor/stats", get(stats))
        .route("/monitor/graph", get(graph))
        .route("/monitor/summary", get(summary))
}

fn projection(state: &Shared, what: &str, tail: usize) -> Value {
    match state.monitor_hook.get() {
        Some(hook) => hook(what, tail),
        None => json!({
            "available": false,
            "note": "框架监测未接入：宿主启动时未安装 monitor",
        }),
    }
}

async fn chains(State(state): State<Shared>) -> Result<Response, ApiError> {
    Ok(util::ok(&projection(&state, "chains", 0)))
}

async fn events(
    State(state): State<Shared>,
    Query(query): Query<MonitorQuery>,
) -> Result<Response, ApiError> {
    let tail = query.tail.unwrap_or(64).clamp(1, 512);
    Ok(util::ok(&projection(&state, "events", tail)))
}

async fn stats(State(state): State<Shared>) -> Result<Response, ApiError> {
    Ok(util::ok(&projection(&state, "stats", 0)))
}

async fn graph(State(state): State<Shared>) -> Result<Response, ApiError> {
    Ok(util::ok(&projection(&state, "graph", 0)))
}

/// 工作台一次取齐：统计 + 最近事件，避免三四个请求同时打到监测面。
async fn summary(State(state): State<Shared>) -> Result<Response, ApiError> {
    let mut payload = projection(&state, "summary", 40);
    if let Some(map) = payload.as_object_mut() {
        map.insert("generatedAt".into(), Value::String(now()));
    }
    Ok(util::ok(&payload))
}
