//! 公共错误模型与请求关联。
//!
//! 所有非 2xx 响应使用 `{"error": {code, message, fieldErrors, requestId, retryable}}`。
//! requestId 由中间件放入 task-local，错误构造处不需要逐层透传。

use std::fmt;

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

tokio::task_local! {
    /// 当前请求的关联标识。中间件包裹整个 handler future。
    pub static REQUEST_ID: String;
}

/// 读取当前请求标识；单元测试或后台任务没有请求上下文时给稳定占位。
pub fn current_request_id() -> String {
    REQUEST_ID
        .try_with(Clone::clone)
        .unwrap_or_else(|_| "req_background".into())
}

/// 读取当前请求标识的值；后台任务返回 None。
pub fn request_id_opt() -> Option<String> {
    REQUEST_ID.try_with(Clone::clone).ok()
}

#[derive(Clone, Debug)]
pub struct FieldError {
    pub field: String,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub field_errors: Vec<FieldError>,
    pub retryable: bool,
    pub request_id: String,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            field_errors: Vec::new(),
            retryable: false,
            request_id: current_request_id(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalidRequest", message)
    }

    pub fn unknown_field(field: &str) -> Self {
        let mut error = Self::new(
            StatusCode::BAD_REQUEST,
            "unknownField",
            format!("请求包含未知字段: {field}"),
        );
        error.field_errors.push(FieldError {
            field: field.to_string(),
            code: "unknownField".into(),
            message: "不接受该字段".into(),
        });
        error
    }

    pub fn validation(field: &str, code: &str, message: impl Into<String>) -> Self {
        let message = message.into();
        let mut error = Self::new(
            StatusCode::BAD_REQUEST,
            "validationFailed",
            format!("{field}: {message}"),
        );
        error.field_errors.push(FieldError {
            field: field.to_string(),
            code: code.to_string(),
            message,
        });
        error
    }

    pub fn not_found(what: impl fmt::Display) -> Self {
        Self::new(StatusCode::NOT_FOUND, "notFound", format!("不存在: {what}"))
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", message)
    }

    pub fn precondition(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::PRECONDITION_FAILED,
            "preconditionFailed",
            message,
        )
    }

    pub fn idempotency_conflict() -> Self {
        Self::new(
            StatusCode::CONFLICT,
            "idempotencyConflict",
            "相同幂等键对应了不同请求",
        )
    }

    pub fn cursor_expired() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "cursorExpired",
            "游标已过期或与当前筛选不一致",
        )
    }

    pub fn not_cancellable() -> Self {
        Self::new(
            StatusCode::CONFLICT,
            "operationNotCancellable",
            "操作已进入不可取消阶段",
        )
    }

    pub fn not_retryable() -> Self {
        Self::new(
            StatusCode::CONFLICT,
            "operationNotRetryable",
            "该操作不可重试",
        )
    }

    pub fn maintenance() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "maintenanceMode",
            "系统处于维护模式，暂不接受写入",
        )
    }

    pub fn dependency(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "dependencyUnavailable",
            message,
        )
    }

    pub fn internal(message: impl Into<String>) -> Self {
        let detail = message.into();
        tracing::error!(detail = %detail, "内部错误");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internalError",
            "内部处理失败，请查看日志",
        )
    }

    pub fn retryable(mut self) -> Self {
        self.retryable = true;
        self
    }

    pub fn body(&self) -> Value {
        let field_errors: Vec<Value> = self
            .field_errors
            .iter()
            .map(|item| {
                json!({
                    "field": item.field,
                    "code": item.code,
                    "message": item.message,
                })
            })
            .collect();
        json!({
            "error": {
                "code": self.code,
                "message": self.message,
                "fieldErrors": field_errors,
                "requestId": self.request_id,
                "retryable": self.retryable,
            }
        })
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let request_id = self.request_id.clone();
        let status = self.status;
        let mut response = (status, Json(self.body())).into_response();
        if let Ok(value) = HeaderValue::from_str(&request_id) {
            response.headers_mut().insert("x-request-id", value);
        }
        if self.retryable {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
        }
        response
    }
}

impl From<assembly::Error> for ApiError {
    fn from(error: assembly::Error) -> Self {
        let detail = error.to_string();
        tracing::warn!(detail = %detail, "能力或装配错误");
        let message = cleanse(&detail);
        let lower = message.to_ascii_lowercase();
        if lower.contains("不存在") || lower.contains("not found") {
            ApiError::not_found(message)
        } else if lower.contains("参数") || lower.contains("无效") || lower.contains("invalid")
        {
            ApiError::validation("request", "invalid", message)
        } else if lower.contains("不可用") || lower.contains("timeout") {
            ApiError::dependency(message).retryable()
        } else {
            ApiError::internal(detail)
        }
    }
}

impl From<config::Error> for ApiError {
    fn from(error: config::Error) -> Self {
        tracing::warn!(detail = %error, "配置校验错误");
        match error {
            config::Error::Invalid(_) => {
                ApiError::validation("request", "invalid", "声明未通过配置校验，请检查字段与引用")
            }
            _ => ApiError::internal(error.to_string()),
        }
    }
}

/// 用户可见的错误信息里不出现物理路径、配置文件名或数据库文件名。
fn cleanse(message: &str) -> String {
    message
        .split_whitespace()
        .map(|token| {
            let lower = token.to_ascii_lowercase();
            if lower.contains(".yaml")
                || lower.contains(".yml")
                || lower.contains(":\\")
                || lower.contains(".db")
                || lower.contains("/sources/")
                || lower.contains("/accessors/")
                || lower.contains("/adapters/")
            {
                "[内部路径]"
            } else {
                token
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl From<strata_client::Error> for ApiError {
    fn from(error: strata_client::Error) -> Self {
        let message = error.to_string();
        let lower = message.to_ascii_lowercase();
        if lower.contains("missing") || lower.contains("not found") {
            ApiError::not_found(message)
        } else {
            ApiError::internal(message)
        }
    }
}
