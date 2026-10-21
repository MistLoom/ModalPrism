//! HTTP 适配：业务逻辑在 `console` crate。

use axum::Json;
use axum::extract::{Path as AxPath, Query, State};
use axum::http::StatusCode;
use console::handlers::{
    AsOfQuery, ConfirmBody, FilePathBody, FileValidateBody, PatchSource, PutFileBody, RecordsQuery,
    TriggerBody,
};
use console::{ConsoleContext, ConsoleError};
use serde_json::Value;

use crate::error::{ApiError, ApiResult};
use crate::ConsoleState;

fn context(state: &ConsoleState) -> ConsoleContext {
    ConsoleContext {
        gateway: state.gateway.clone(),
        http_addr: state.http_addr.clone(),
    }
}

fn map_err(error: ConsoleError) -> ApiError {
    ApiError {
        status: StatusCode::from_u16(error.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        code: error.code,
        message: error.message,
    }
}

async fn run<F>(_state: &ConsoleState, f: F) -> ApiResult<Json<Value>>
where
    F: std::future::Future<Output = Result<Value, ConsoleError>>,
{
    f.await.map(Json).map_err(map_err)
}

pub async fn meta(State(state): State<ConsoleState>, Query(q): Query<AsOfQuery>) -> ApiResult<Json<Value>> {
    run(&state, console::handlers::meta(&context(&state), q)).await
}

pub async fn timeline(State(state): State<ConsoleState>) -> ApiResult<Json<Value>> {
    run(&state, console::handlers::timeline(&context(&state))).await
}

pub async fn overview(
    State(state): State<ConsoleState>,
    Query(q): Query<AsOfQuery>,
) -> ApiResult<Json<Value>> {
    run(&state, console::handlers::overview(&context(&state), q)).await
}

pub async fn sources_list(
    State(state): State<ConsoleState>,
    Query(q): Query<AsOfQuery>,
) -> ApiResult<Json<Value>> {
    run(&state, console::handlers::sources_list(&context(&state), q)).await
}

pub async fn source_get(
    State(state): State<ConsoleState>,
    AxPath(id): AxPath<String>,
    Query(q): Query<AsOfQuery>,
) -> ApiResult<Json<Value>> {
    run(
        &state,
        console::handlers::source_get(&context(&state), id, q),
    )
    .await
}

pub async fn source_trigger(
    State(state): State<ConsoleState>,
    AxPath(id): AxPath<String>,
    body: Option<Json<TriggerBody>>,
) -> ApiResult<Json<Value>> {
    run(
        &state,
        console::handlers::source_trigger(&context(&state), id, body.map(|b| b.0)),
    )
    .await
}

pub async fn source_patch(
    State(state): State<ConsoleState>,
    AxPath(id): AxPath<String>,
    Json(body): Json<PatchSource>,
) -> ApiResult<Json<Value>> {
    run(&state, console::handlers::source_patch(&context(&state), id, body)).await
}

pub async fn records_list(
    State(state): State<ConsoleState>,
    Query(q): Query<RecordsQuery>,
) -> ApiResult<Json<Value>> {
    run(&state, console::handlers::records_list(&context(&state), q)).await
}

pub async fn record_get(
    State(state): State<ConsoleState>,
    AxPath(id): AxPath<String>,
    Query(q): Query<AsOfQuery>,
) -> ApiResult<Json<Value>> {
    run(&state, console::handlers::record_get(&context(&state), id, q)).await
}

pub async fn governance_egress(
    State(state): State<ConsoleState>,
    Query(q): Query<AsOfQuery>,
) -> ApiResult<Json<Value>> {
    run(&state, console::handlers::governance_egress(&context(&state), q)).await
}

pub async fn reset_circuit(
    State(state): State<ConsoleState>,
    AxPath(host): AxPath<String>,
    body: Option<Json<ConfirmBody>>,
) -> ApiResult<Json<Value>> {
    run(
        &state,
        console::handlers::reset_circuit(&context(&state), host, body.map(|b| b.0)),
    )
    .await
}

pub async fn config_summary(State(state): State<ConsoleState>) -> ApiResult<Json<Value>> {
    run(&state, console::handlers::config_summary(&context(&state))).await
}

pub async fn config_validate(State(state): State<ConsoleState>) -> ApiResult<Json<Value>> {
    run(&state, console::handlers::config_validate(&context(&state))).await
}

pub async fn config_reload(State(state): State<ConsoleState>) -> ApiResult<Json<Value>> {
    run(&state, console::handlers::config_reload(&context(&state))).await
}

pub async fn config_files_list(State(state): State<ConsoleState>) -> ApiResult<Json<Value>> {
    run(&state, console::handlers::config_files_list(&context(&state))).await
}

pub async fn config_file_get(
    State(state): State<ConsoleState>,
    AxPath(path): AxPath<String>,
) -> ApiResult<Json<Value>> {
    run(&state, console::handlers::config_file_get(&context(&state), path)).await
}

pub async fn config_file_validate_text(
    State(state): State<ConsoleState>,
    Json(body): Json<FileValidateBody>,
) -> ApiResult<Json<Value>> {
    run(
        &state,
        console::handlers::config_file_validate_text(&context(&state), body),
    )
    .await
}

pub async fn config_file_put(
    State(state): State<ConsoleState>,
    AxPath(path): AxPath<String>,
    Json(body): Json<PutFileBody>,
) -> ApiResult<Json<Value>> {
    run(
        &state,
        console::handlers::config_file_put(&context(&state), path, body),
    )
    .await
}

pub async fn config_file_rollback(
    State(state): State<ConsoleState>,
    Json(body): Json<FilePathBody>,
) -> ApiResult<Json<Value>> {
    run(&state, console::handlers::config_file_rollback(&context(&state), body)).await
}
