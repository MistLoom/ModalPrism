//! ModalPrism 领域公共 API。
//!
//! 这是控制台与受控集成唯一使用的 HTTP 契约：`/api/v1` 的领域资源、
//! 持久化 Operation、统一错误、请求关联和观测流。内部 Ability 与配置目录
//! 不出现在协议里；Node.js 只托管 SPA 并原样代理本服务。

mod bindings;
mod catalog;
mod collection;
mod content;
mod error;
mod maintenance;
mod observe;
mod operations;
mod preconditions;
mod production;
mod projects;
mod publication;
pub mod search;
mod settings;
mod store;
mod surface;
mod system;
mod util;
mod wizard;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use assembly::Services;
use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use tower::ServiceExt;

pub use error::{ApiError, ApiError as Error};
pub use store::Store;

use crate::error::REQUEST_ID;

/// HTTP 服务版本，随公开协议冻结。
pub const API_VERSION: &str = "1.0.0";

/// 结构化解析钩子：宿主（collector）注入 rawprism 调用；None 时 /parse 返回 412。
pub type ParseHookFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, String>> + Send>>;
pub type ParseHook = Arc<dyn Fn(&Value) -> ParseHookFuture + Send + Sync>;

#[derive(Clone)]
pub struct AppState {
    pub services: Arc<Services>,
    pub store: Store,
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub http_addr: SocketAddr,
    pub started_at: String,
    pub maintenance: Arc<AtomicBool>,
    pub resetting: Arc<AtomicBool>,
    pub retention_at: Arc<AtomicU64>,
    pub parse_hook: ParseHookOption,
}

pub type ParseHookOption = Option<ParseHook>;

pub type Shared = Arc<AppState>;

impl AppState {
    pub async fn open(
        services: Arc<Services>,
        config_dir: &Path,
        data_dir: &Path,
        http_addr: SocketAddr,
    ) -> Result<Shared, ApiError> {
        let store = Store::open(data_dir).await?;
        let state = Arc::new(Self {
            services,
            store,
            config_dir: config_dir.to_path_buf(),
            data_dir: data_dir.to_path_buf(),
            http_addr,
            started_at: now(),
            maintenance: Arc::new(AtomicBool::new(false)),
            resetting: Arc::new(AtomicBool::new(false)),
            retention_at: Arc::new(AtomicU64::new(0)),
            parse_hook: None,
        });
        settings::seed(&state).await?;
        projects::seed(&state).await?;
        maintenance::ensure_factory_snapshot(&state).await?;
        observe::append_audit(
            &state,
            "system.start",
            "succeeded",
            "领域 API 启动",
            json!({"version": API_VERSION}),
            None,
        )
        .await?;
        let resume = Arc::clone(&state);
        tokio::spawn(async move {
            crate::operations::recover_interrupted(resume).await;
        });
        let retention = Arc::clone(&state);
        tokio::spawn(async move {
            crate::observe::maybe_retention(&retention).await;
        });
        Ok(state)
    }

    pub fn in_maintenance(&self) -> bool {
        self.maintenance.load(Ordering::Relaxed)
    }

    pub fn enter_maintenance(&self) {
        self.maintenance.store(true, Ordering::Relaxed);
    }

    pub fn leave_maintenance(&self) {
        self.maintenance.store(false, Ordering::Relaxed);
    }

    /// 同一时刻只允许一个清空/恢复出厂事务。
    pub fn try_begin_reset(&self) -> bool {
        self.resetting
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    pub fn end_reset(&self) {
        self.resetting.store(false, Ordering::SeqCst);
    }

    pub fn reset_in_progress(&self) -> bool {
        self.resetting.load(Ordering::SeqCst)
    }
}

/// 组装 `/api/v1` 路由。
pub fn router(state: Shared) -> Router {
    let api = Router::new()
        .merge(system::routes())
        .merge(catalog::routes())
        .merge(bindings::routes())
        .merge(collection::routes())
        .merge(content::routes())
        .merge(production::routes())
        .merge(publication::routes())
        .merge(projects::routes())
        .merge(wizard::routes())
        .merge(search::routes())
        .merge(surface::routes())
        .merge(preconditions::routes())
        .merge(settings::routes())
        .merge(operations::routes())
        .merge(observe::routes())
        .merge(maintenance::routes())
        .with_state(Arc::clone(&state))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            idempotency,
        ))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            maintenance_guard,
        ))
        .layer(middleware::from_fn_with_state(state, associate));
    let routes = Router::new().nest("/api/v1", api);
    // 动作路径用 `:action` 表达，但 matchit 不接受参数段带冒号；
    // Router::layer 发生在路径匹配之后，因此只能由 fallback 先重写再二次分发。
    let dispatch = routes
        .clone()
        .fallback(|| async { ApiError::not_found("接口").into_response() });
    routes.fallback(move |request: Request| {
        let dispatch = dispatch.clone();
        async move {
            let request_id = format!("req_{}", ulid::Ulid::new().to_string().to_lowercase());
            REQUEST_ID
                .scope(request_id, async move {
                    let Some(request) = rewrite_action_request(request) else {
                        return ApiError::not_found("接口").into_response();
                    };
                    match dispatch.oneshot(request).await {
                        Ok(response) => response,
                        Err(error) => match error {},
                    }
                })
                .await
        }
    })
}

/// 把路径最后一段的 `:action` 改写成 `/action`。
fn rewrite_action_request(mut request: Request) -> Option<Request> {
    let uri = request.uri().clone();
    let (head, last) = uri.path().rsplit_once('/')?;
    let (base, action) = last.split_once(':')?;
    if base.is_empty() || action.is_empty() {
        return None;
    }
    let query = uri
        .query()
        .map(|value| format!("?{value}"))
        .unwrap_or_default();
    let rewritten = format!("{head}/{base}/{action}{query}").parse().ok()?;
    let mut parts = uri.into_parts();
    parts.path_and_query = Some(rewritten);
    *request.uri_mut() = axum::http::Uri::from_parts(parts).ok()?;
    Some(request)
}

/// 请求关联：生成或接受 X-Request-Id，写入 task-local 与响应头，并记录访问日志。
async fn associate(State(state): State<Shared>, mut request: Request, next: Next) -> Response {
    let incoming = request
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let request_id =
        incoming.unwrap_or_else(|| format!("req_{}", ulid::Ulid::new().to_string().to_lowercase()));
    request
        .extensions_mut()
        .insert(RequestId(request_id.clone()));
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let started = Instant::now();
    let response = REQUEST_ID
        .scope(request_id.clone(), next.run(request))
        .await;
    let ms = started.elapsed().as_millis() as u64;
    let status = response.status().as_u16();
    let level = if status >= 500 {
        "error"
    } else if status >= 400 {
        "warn"
    } else {
        "info"
    };
    let record = json!({
        "timestamp": now(),
        "level": level,
        "kind": "http",
        "message": format!("{method} {path} {status}"),
        "requestId": request_id,
        "operationId": Value::Null,
        "taskId": Value::Null,
        "sourceId": Value::Null,
        "contentId": Value::Null,
        "runId": Value::Null,
        "attemptId": Value::Null,
        "workflowVersionId": Value::Null,
        "publicationId": Value::Null,
        "deliveryId": Value::Null,
        "configGeneration": state.services.config_generation() as i64,
        "data": {"method": method.as_str(), "path": path, "status": status, "durationMs": ms},
    });
    let _ = state.store.append_record(&record, "logs").await;
    crate::observe::maybe_retention(&state).await;
    let mut response = response;
    if let Ok(value) = axum::http::HeaderValue::from_str(&request_id) {
        response.headers_mut().insert("x-request-id", value);
    }
    response
}

#[derive(Clone, Debug)]
pub struct RequestId(pub String);

/// 维护模式拒绝写入；维护端点自身与操作取消/重试保持可达。
async fn maintenance_guard(State(state): State<Shared>, request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let writable = method != Method::GET && method != Method::HEAD && method != Method::OPTIONS;
    // 维护期间只保留取消/重试入口；新的清空、预演与写入全部阻断。
    let allowed = path.contains(":cancel")
        || path.contains(":retry")
        || path.contains("/cancel")
        || path.contains("/retry");
    if writable && state.in_maintenance() && !allowed {
        return ApiError::maintenance().into_response();
    }
    next.run(request).await
}

/// 幂等重放：只处理带 Idempotency-Key 的写请求，成功响应原样回放。
async fn idempotency(State(state): State<Shared>, request: Request, next: Next) -> Response {
    let method = request.method().clone();
    if method == Method::GET || method == Method::HEAD || method == Method::OPTIONS {
        return next.run(request).await;
    }
    let Some(key) = request
        .headers()
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
    else {
        return next.run(request).await;
    };
    if key.len() < 8 || key.len() > 200 {
        return ApiError::validation("Idempotency-Key", "length", "长度需要在 8 到 200 之间")
            .into_response();
    }
    let (parts, body) = request.into_parts();
    let bytes = match axum::body::to_bytes(body, 32 * 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(_) => return ApiError::invalid("读取请求体失败").into_response(),
    };
    let fingerprint = util::fingerprint(method.as_str(), &parts.uri.to_string(), &bytes);
    match state.store.reserve_idempotent(&key, &fingerprint).await {
        Ok(true) => {}
        Ok(false) => match state.store.idempotent(&key, &fingerprint).await {
            Ok(Some((status, body))) if status != 0 => {
                let mut response = Response::new(Body::from(body.to_string()));
                *response.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::OK);
                response.headers_mut().insert(
                    header::CONTENT_TYPE,
                    axum::http::HeaderValue::from_static("application/json"),
                );
                response.headers_mut().insert(
                    "idempotent-replayed",
                    axum::http::HeaderValue::from_static("true"),
                );
                return response;
            }
            Ok(Some(_)) | Ok(None) => {
                return ApiError::conflict("相同 Idempotency-Key 的请求正在处理").into_response();
            }
            Err(error) => return error.into_response(),
        },
        Err(error) => return error.into_response(),
    }
    let request = Request::from_parts(parts, Body::from(bytes));
    let response = next.run(request).await;
    let status = response.status();
    if status.is_success() {
        let (parts, body) = response.into_parts();
        let bytes = match axum::body::to_bytes(body, 32 * 1024 * 1024).await {
            Ok(bytes) => bytes,
            Err(_) => return ApiError::internal("读取响应失败").into_response(),
        };
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        let _ = state
            .store
            .remember_idempotent(&key, &fingerprint, status.as_u16(), &value)
            .await;
        let response = Response::from_parts(parts, Body::from(bytes));
        return response;
    }
    let _ = state.store.forget_idempotent(&key).await;
    response
}

pub(crate) fn now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}

pub(crate) fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", ulid::Ulid::new().to_string().to_lowercase())
}
