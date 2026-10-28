use std::sync::Arc;

use ability_support::RateLimiter;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use frame_core::{
    CatalogEntry, DefaultHttpController, Gateway, InvokeMask, InvokeResponse, LocalsScope,
    ProtocolController, resolve_target,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::ApiError;

#[derive(Clone)]
struct AppState {
    gateway: Arc<dyn Gateway>,
    controller: Arc<dyn ProtocolController>,
    rate_limiter: Option<Arc<dyn RateLimiter>>,
    auth_token: Option<Arc<String>>,
}

pub fn build_router(gateway: Arc<dyn Gateway>) -> Router {
    build_router_with(gateway, Arc::new(DefaultHttpController), None, None)
}

pub fn build_router_with(
    gateway: Arc<dyn Gateway>,
    controller: Arc<dyn ProtocolController>,
    rate_limiter: Option<Arc<dyn RateLimiter>>,
    auth_token: Option<Arc<String>>,
) -> Router {
    let state = AppState {
        gateway,
        controller,
        rate_limiter,
        auth_token,
    };
    // `/health` 豁免鉴权；其余路由在 handler 内校验 Bearer。
    Router::new()
        .route("/health", get(health))
        .route("/v1/abilities", get(list_abilities))
        .route("/v1/abilities/{target}", get(get_ability))
        .route("/v1/abilities/{target}/invoke", post(invoke_ability))
        .with_state(state)
}

/// Bearer 校验：配置了令牌时要求 `Authorization: Bearer <token>` 匹配。
fn check_auth(state: &AppState, headers: &HeaderMap) -> Result<(), Response> {
    let Some(expected) = &state.auth_token else {
        return Ok(());
    };
    let provided = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if provided == Some(expected.as_str()) {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": {
                    "type": "unauthorized",
                    "message": "缺少或无效的 Bearer 凭证",
                }
            })),
        )
            .into_response())
    }
}

async fn health() -> Json<Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

async fn list_abilities(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<CatalogEntry>>, Response> {
    check_auth(&state, &headers)?;
    Ok(Json(state.controller.catalog(state.gateway.as_ref())))
}

async fn get_ability(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(target): Path<String>,
) -> Result<Json<CatalogEntry>, Response> {
    check_auth(&state, &headers)?;
    let (inst, method) = match resolve_target(&target, &state.gateway.catalog()) {
        Ok(v) => v,
        Err(err) => return Err(ApiError(err).into_response()),
    };
    let meta = state
        .gateway
        .get_entry(&inst, &method)
        .ok_or_else(|| ApiError(frame_core::Error::not_found(target.clone())).into_response())?;
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
    /// 调用链内实例可见性；跨进程客户端经 REST 转发时透传。
    #[serde(default, skip_serializing_if = "InvokeMask::is_default")]
    pub invoke_mask: InvokeMask,
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
    headers: HeaderMap,
    Path(target): Path<String>,
    Query(q): Query<InvokeQuery>,
    Json(body): Json<InvokeBody>,
) -> Result<Json<InvokeOut>, Response> {
    check_auth(&state, &headers)?;
    let raw = body
        .method
        .as_ref()
        .or(q.method.as_ref())
        .map(|m| format!("{target}.{m}"))
        .unwrap_or(target);
    let (inst, method) =
        resolve_target(&raw, &state.gateway.catalog()).map_err(|e| ApiError(e).into_response())?;
    // 限流准入：按目标实例计桶；超限返回 429（rate_limited），可稍后重试。
    if let Some(limiter) = &state.rate_limiter {
        if !limiter.try_acquire(inst.as_str()) {
            return Err(ApiError(frame_core::Error::rate_limited(format!(
                "实例 {inst} 触发限流"
            )))
            .into_response());
        }
    }
    let args = state
        .controller
        .normalize_args(body.args, None)
        .map_err(|e| ApiError(e).into_response())?;
    let request = state
        .controller
        .build_request(inst, method, args, body.session_id)
        .with_invoke_mask(body.invoke_mask);
    let response = state
        .gateway
        .invoke(request)
        .await
        .map_err(|e| ApiError(e).into_response())?;
    Ok(Json(InvokeOut { response }))
}
