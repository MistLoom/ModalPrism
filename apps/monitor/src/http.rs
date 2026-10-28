//! 可选 HTTP 监测面。默认关闭；开启时本机绑定，与 mgmt 端口分离。

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::routing::{get, post};
use serde_json::{Value, json};

use crate::MonitorApp;
use ability_support::Result;

/// 构建监测路由（可并入入口应用自己的 Router）。
pub fn router(app: Arc<MonitorApp>) -> axum::Router {
    axum::Router::new()
        .route("/monitor/chains", get(chains))
        .route("/monitor/events", post(events))
        .route("/monitor/log", post(log))
        .route("/monitor/stats", get(stats))
        .route("/monitor/graph", get(graph))
        .with_state(app)
}

async fn chains(State(app): State<Arc<MonitorApp>>) -> Json<Value> {
    Json(json!({ "chains": app.chains() }))
}

#[derive(serde::Deserialize, Default)]
struct TailArgs {
    tail: Option<usize>,
}

async fn events(State(app): State<Arc<MonitorApp>>, args: Option<Json<TailArgs>>) -> Json<Value> {
    let tail = args.and_then(|Json(a)| a.tail).unwrap_or(64).max(1);
    let evs = app.recent_events();
    let start = evs.len().saturating_sub(tail);
    Json(json!({ "events": &evs[start..] }))
}

async fn log(State(app): State<Arc<MonitorApp>>, args: Option<Json<TailArgs>>) -> Json<Value> {
    let tail = args.and_then(|Json(a)| a.tail).unwrap_or(64).max(1);
    match app.tail_log(tail) {
        Ok(lines) => Json(json!({ "lines": lines })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

async fn stats(State(app): State<Arc<MonitorApp>>) -> Json<Value> {
    Json(json!(app.stats()))
}

async fn graph(State(app): State<Arc<MonitorApp>>) -> Json<Value> {
    Json(json!(app.chain_graph()))
}

/// 按配置启动 HTTP 监测面（`http.enabled = true` 时）。
pub async fn serve_if_enabled(app: Arc<MonitorApp>) -> Result<Option<std::net::SocketAddr>> {
    if !app.config.http.enabled {
        return Ok(None);
    }
    let addr: std::net::SocketAddr = app
        .config
        .http
        .bind
        .parse()
        .map_err(|e| ability_support::Error::config(format!("monitor.http.bind 无效: {e}")))?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| ability_support::Error::io(format!("监测 HTTP 绑定失败 {addr}: {e}")))?;
    let local = listener.local_addr().ok();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router(app)).await;
    });
    Ok(local)
}
