use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use frame_core::{
    CatalogEntry, DefaultHttpController, Gateway, InvokeResponse, LocalsScope, ProtocolController,
    resolve_target,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ApiError;

#[derive(Clone)]
struct AppState {
    gateway: Arc<dyn Gateway>,
    controller: Arc<dyn ProtocolController>,
}

pub fn build_router(gateway: Arc<dyn Gateway>) -> Router {
    build_router_with(gateway, Arc::new(DefaultHttpController))
}

pub fn build_router_with(
    gateway: Arc<dyn Gateway>,
    controller: Arc<dyn ProtocolController>,
) -> Router {
    let state = AppState {
        gateway,
        controller,
    };
    Router::new()
        .route("/health", get(health))
        .route("/v1/abilities", get(list_abilities))
        .route("/v1/abilities/{target}", get(get_ability))
        .route("/v1/abilities/{target}/invoke", post(invoke_ability))
        .with_state(state)
}

async fn health() -> Json<Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

async fn list_abilities(State(state): State<AppState>) -> Json<Vec<CatalogEntry>> {
    Json(state.controller.catalog(state.gateway.as_ref()))
}

async fn get_ability(
    State(state): State<AppState>,
    Path(target): Path<String>,
) -> Result<Json<CatalogEntry>, ApiError> {
    let (inst, method) = resolve_target(&target, &state.gateway.catalog())?;
    let meta = state
        .gateway
        .get_entry(&inst, &method)
        .ok_or_else(|| frame_core::Error::not_found(target))?;
    Ok(Json(meta))
}

#[derive(Debug, Deserialize)]
pub struct InvokeBody {
    #[serde(default)]
    pub args: Value,
    #[serde(default)]
    pub session_id: Option<LocalsScope>,
    #[serde(default)]
    pub method: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InvokeQuery {
    method: Option<String>,
}

#[derive(Debug, Serialize)]
struct InvokeOut {
    #[serde(flatten)]
    response: InvokeResponse,
}

async fn invoke_ability(
    State(state): State<AppState>,
    Path(target): Path<String>,
    Query(q): Query<InvokeQuery>,
    Json(body): Json<InvokeBody>,
) -> Result<Json<InvokeOut>, ApiError> {
    let raw = body
        .method
        .as_ref()
        .or(q.method.as_ref())
        .map(|m| format!("{target}.{m}"))
        .unwrap_or(target);
    let (inst, method) = resolve_target(&raw, &state.gateway.catalog())?;
    let response = state
        .controller
        .invoke(
            state.gateway.as_ref(),
            inst,
            method,
            body.args,
            None,
            body.session_id,
        )
        .await?;
    Ok(Json(InvokeOut { response }))
}
