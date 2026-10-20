//! 环回 Host/Origin 与写请求 CSRF 防护。

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::error::ApiError;

const WRITE_HEADER: &str = "x-modalprism-console";

pub async fn guard_write_request(request: Request<Body>, next: Next) -> Response {
    let method = request.method().clone();
    if !is_mutating(&method) {
        return next.run(request).await;
    }
    if let Err(err) = check_host(request.headers()) {
        return err.into_response();
    }
    if let Err(err) = check_origin(request.headers()) {
        return err.into_response();
    }
    if let Err(err) = check_content_type(request.headers()) {
        return err.into_response();
    }
    next.run(request).await
}

fn is_mutating(method: &Method) -> bool {
    matches!(
        method,
        &Method::POST | &Method::PUT | &Method::PATCH | &Method::DELETE
    )
}

fn check_host(headers: &axum::http::HeaderMap) -> Result<(), ApiError> {
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if host.is_empty() {
        return Ok(());
    }
    let host_only = host.split(':').next().unwrap_or(host);
    if is_loopback_host(host_only) {
        Ok(())
    } else {
        Err(ApiError {
            status: StatusCode::FORBIDDEN,
            code: "forbidden",
            message: "仅允许本机访问".into(),
        })
    }
}

fn check_origin(headers: &axum::http::HeaderMap) -> Result<(), ApiError> {
    let origin = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok());
    if origin.is_none() {
        return Ok(());
    }
    let origin = origin.unwrap();
    let ok = origin.starts_with("http://127.0.0.1:")
        || origin.starts_with("http://localhost:")
        || origin.starts_with("http://[::1]:")
        || origin == "http://127.0.0.1"
        || origin == "http://localhost"
        || origin == "http://[::1]";
    if ok {
        Ok(())
    } else {
        Err(ApiError {
            status: StatusCode::FORBIDDEN,
            code: "forbidden",
            message: "来源地址无效".into(),
        })
    }
}

fn check_content_type(headers: &axum::http::HeaderMap) -> Result<(), ApiError> {
    if headers.contains_key(WRITE_HEADER) {
        return Ok(());
    }
    let ct = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if ct.starts_with("application/json") {
        Ok(())
    } else {
        Err(ApiError {
            status: StatusCode::FORBIDDEN,
            code: "forbidden",
            message: "写请求需要 JSON Content-Type 或控制台请求头".into(),
        })
    }
}

pub fn is_loopback_host(host: &str) -> bool {
    host == "127.0.0.1" || host.eq_ignore_ascii_case("localhost") || host == "[::1]"
}
