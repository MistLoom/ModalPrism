//! 控制台后端。
//!
//! 职责只有两件：把采集进程的运维面聚合到同源 `/api`，以及托管前端产物。
//! 界面只认识这里定义的接口，不需要知道运维面如何分片。

pub mod assets;
pub mod error;
pub mod files;
pub mod ops;

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};

use error::ApiError;
use ops::Ops;

pub struct Console {
    ops: Ops,
    web: PathBuf,
    config_dir: PathBuf,
}

pub fn router(ops: Ops, web: PathBuf, config_dir: PathBuf) -> Router {
    let console = Arc::new(Console {
        ops,
        web,
        config_dir,
    });
    Router::new()
        .route("/api/health", get(health))
        .route("/api/ops/{instance}/{method}", post(invoke))
        .route("/api/config/files", get(config_files))
        .route(
            "/api/config/file",
            get(config_file_read)
                .put(config_file_write)
                .delete(config_file_delete),
        )
        .fallback(get(assets::serve))
        .with_state(console)
}

async fn config_files(State(console): State<Arc<Console>>) -> Result<Json<Value>, ApiError> {
    files::list(&console.config_dir)
}

async fn config_file_read(
    State(console): State<Arc<Console>>,
    Query(query): Query<files::FileQuery>,
) -> Result<Json<Value>, ApiError> {
    files::read(&console.config_dir, &query.path)
}

async fn config_file_write(
    State(console): State<Arc<Console>>,
    Json(body): Json<files::FileBody>,
) -> Result<Json<Value>, ApiError> {
    files::write(&console.config_dir, &body)
}

async fn config_file_delete(
    State(console): State<Arc<Console>>,
    Query(query): Query<files::FileQuery>,
) -> Result<Json<Value>, ApiError> {
    files::delete(&console.config_dir, &query.path)
}

/// 连接探针。界面用它决定是显示数据还是显示唤醒引导。
async fn health(State(console): State<Arc<Console>>) -> Json<Value> {
    let (online, latency, message) = console.ops.probe().await;
    Json(json!({
        "online": online,
        "latencyMs": latency,
        "message": message,
        "endpoint": console.ops.base(),
    }))
}

async fn invoke(
    State(console): State<Arc<Console>>,
    Path((instance, method)): Path<(String, String)>,
    Json(args): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    if !allowed(&instance, &method) {
        return Err(ApiError::missing("该能力"));
    }
    let data = console.ops.call(&instance, &method, args).await?;
    Ok(Json(json!({ "data": data })))
}

/// 允许界面调用的方法。白名单来自 `surface`，此处不重复声明。
fn allowed(instance: &str, method: &str) -> bool {
    surface::exposes(surface::CONSOLE, instance, method)
}
