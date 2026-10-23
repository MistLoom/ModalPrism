//! 控制台后端的错误形状。面向界面的消息不携带后端术语。

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::ops::OpsError;

/// 一次接口失败。`hint` 说明用户可以做什么，`message` 说明发生了什么。
#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
    pub hint: Option<String>,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            hint: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn missing(what: &str) -> Self {
        Self::new(StatusCode::NOT_FOUND, format!("{what}不存在"))
    }
}

impl From<OpsError> for ApiError {
    fn from(error: OpsError) -> Self {
        match error {
            OpsError::Unreachable(detail) => {
                Self::new(StatusCode::SERVICE_UNAVAILABLE, "采集进程未响应").with_hint(format!(
                    "确认 modalprism-collector 已启动，并且运维端口可访问：{detail}"
                ))
            }
            OpsError::Rejected(message) => Self::new(StatusCode::UNPROCESSABLE_ENTITY, message)
                .with_hint("该操作被采集进程拒绝，详见配置与健康状态"),
            OpsError::Shape(detail) => {
                Self::new(StatusCode::BAD_GATEWAY, "采集进程返回了无法识别的数据").with_hint(detail)
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(json!({
            "error": {
                "message": self.message,
                "hint": self.hint,
            }
        }));
        (self.status, body).into_response()
    }
}
