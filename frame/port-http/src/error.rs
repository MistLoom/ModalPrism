use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use frame_core::{Error, ErrorKind};
use serde_json::json;

pub struct ApiError(pub Error);

impl From<Error> for ApiError {
    fn from(value: Error) -> Self {
        Self(value)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.0.kind() {
            ErrorKind::NotFound | ErrorKind::SessionNotFound => StatusCode::NOT_FOUND,
            ErrorKind::InvalidInput | ErrorKind::SessionRequired | ErrorKind::Config => {
                StatusCode::BAD_REQUEST
            }
            ErrorKind::Failed
            | ErrorKind::Dependency
            | ErrorKind::Unavailable
            | ErrorKind::Upstream
            | ErrorKind::ToolchainMissing
            | ErrorKind::ArtifactMissing
            | ErrorKind::HandshakeFailed => StatusCode::UNPROCESSABLE_ENTITY,
            ErrorKind::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            ErrorKind::Io => StatusCode::INTERNAL_SERVER_ERROR,
        };

        // 保留原始错误消息，不覆盖为泛化文案
        let body = Json(json!({
            "error": {
                "type": self.0.code(),
                "message": self.0.to_string(),
            }
        }));
        (status, body).into_response()
    }
}
