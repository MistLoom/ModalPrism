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
        } else if lower.contains("不可采集") {
            // 缺采集入口是请求本身满足不了，不是服务端故障：给出用户能照着做的指引。
            ApiError::validation(
                "sourceId",
                "sourceNotCollectable",
                "该信源没有可用的采集入口，请先补齐访问器或用推送端提交",
            )
        } else if lower.contains("不可用") || lower.contains("timeout") {
            ApiError::dependency(message).retryable()
        } else {
            ApiError::internal(detail)
        }
    }
}

impl From<config::Error> for ApiError {
    fn from(error: config::Error) -> Self {
        match error {
            config::Error::Invalid(diagnostics) => invalid_declaration(diagnostics),
            other => {
                tracing::warn!(detail = %other, "配置读写错误");
                ApiError::internal(other.to_string())
            }
        }
    }
}

/// 一次响应里最多回几条诊断：再多会把列表刷成噪音。
const MAX_DECLARATION_DIAGNOSTICS: usize = 8;

/// 配置校验失败要回具体诊断。
///
/// 校验是全库的，出错的那份声明未必是刚写入的那份；只回一句「未通过配置校验」，
/// 调用方无从知道是哪一份声明的哪个引用悬空，只能反复试。物理路径仍然不外泄：
/// 定位统一换成声明 id 形态。
fn invalid_declaration(diagnostics: config::Diagnostics) -> ApiError {
    let items = diagnostics.0;
    let shown: Vec<String> = items
        .iter()
        .take(MAX_DECLARATION_DIAGNOSTICS)
        .map(|item| {
            let message = cleanse(&item.message);
            let where_ = cleanse(&declaration_ref(&item.file));
            if where_.is_empty() {
                message
            } else {
                format!("{where_}: {message}")
            }
        })
        .collect();
    let mut message = shown.first().cloned().unwrap_or_else(|| "声明未通过配置校验".into());
    if items.len() > shown.len() {
        message = format!("{message}（另有 {} 条）", items.len() - shown.len());
    }
    let mut error = ApiError::new(StatusCode::BAD_REQUEST, "validationFailed", message);
    error.field_errors = items
        .iter()
        .take(MAX_DECLARATION_DIAGNOSTICS)
        .map(|item| FieldError {
            field: cleanse(&declaration_ref(&item.file)),
            code: "invalidDeclaration".into(),
            message: cleanse(&item.message),
        })
        .collect();
    tracing::warn!(detail = %items.iter().map(|item| item.to_string()).collect::<Vec<_>>().join("; "), "配置校验错误");
    error
}

/// 诊断位置换成声明标识：`sources/foo.yaml` 回 `source/foo`，已是声明 id 的原样返回。
/// 认不出的形态（文件系统路径等）一律回 `declaration`，不让物理路径泄进字段名。
fn declaration_ref(file: &str) -> String {
    let file = file.trim().trim_start_matches('/');
    // 没有来源文件的诊断（程序侧注入的声明、无 origin 的 schema 失败）也要有个能定位的字段名。
    if file.is_empty() || file.contains('\\') {
        return "declaration".to_string();
    }
    let Some((folder, name)) = file.split_once('/') else {
        return file.to_string();
    };
    // 单数目录开头即声明 id 形态（source/x、adapter/x），原样返回。
    if folder == "source" || folder == "adapter" {
        return file.to_string();
    }
    // 复数目录才是文件形态：sources/foo.yaml → source/foo；其余不是声明
    // （如 schema 目录的绝对路径），给不出可用定位，统一回 declaration。
    let singular = folder.trim_end_matches('s');
    if (singular != "source" && singular != "adapter") || name.is_empty() {
        return "declaration".to_string();
    }
    match name.strip_suffix(".yaml").or_else(|| name.strip_suffix(".yml")) {
        Some(stem) if !stem.is_empty() => format!("{singular}/{stem}"),
        // 没有后缀但已是 `sources/demo` 这类形态的，按单数目录回写。
        None if !name.contains('/') => format!("{singular}/{name}"),
        _ => "declaration".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn not_collectable_source_is_a_validation_error() {
        let error = ApiError::from(assembly::Error::new("信源不可采集"));
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        let body = error.body();
        assert_eq!(body["error"]["code"], "validationFailed");
        assert_eq!(body["error"]["fieldErrors"][0]["field"], "sourceId");
        assert_eq!(
            body["error"]["fieldErrors"][0]["code"],
            "sourceNotCollectable"
        );
        assert!(body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("采集入口"));
    }

    #[test]
    fn unknown_assembly_error_stays_internal() {
        let error = ApiError::from(assembly::Error::new("磁盘写入失败"));
        assert_eq!(error.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(error.body()["error"]["code"], "internalError");
    }

    #[test]
    fn config_diagnostics_reach_the_caller() {
        let error = ApiError::from(config::Error::Invalid(config::Diagnostics(vec![
            config::Diagnostic {
                file: "sources/demo.yaml".into(),
                line: 0,
                message: "引用不存在: adapter/ghost".into(),
            },
        ])));
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        let body = error.body();
        assert_eq!(body["error"]["code"], "validationFailed");
        assert_eq!(body["error"]["fieldErrors"][0]["field"], "source/demo");
        assert_eq!(
            body["error"]["fieldErrors"][0]["code"],
            "invalidDeclaration"
        );
        // 悬空的引用名必须回显：这是调用方唯一能照着改的信息。
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("adapter/ghost")
        );
    }

    #[test]
    fn config_read_error_stays_internal() {
        let error = ApiError::from(config::Error::Read {
            path: "D:\\data\\config\\system.yaml".into(),
            source: std::io::Error::other("denied"),
        });
        assert_eq!(error.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(!error.body()["error"]["message"].as_str().unwrap().contains("data"));
    }

    #[test]
    fn declaration_ref_maps_files_to_module_ids() {
        assert_eq!(declaration_ref("sources/demo.yaml"), "source/demo");
        assert_eq!(declaration_ref("adapters/html-list.yml"), "adapter/html-list");
        assert_eq!(declaration_ref("source/demo"), "source/demo");
        // 没有来源文件的诊断也给个可用的字段名，不让 fieldErrors 出现空字段。
        assert_eq!(declaration_ref(""), "declaration");
        // 文件系统路径不能泄进字段名：schema 目录读取失败记的是绝对路径。
        assert_eq!(declaration_ref("D:\\data\\config\\schemas"), "declaration");
        assert_eq!(declaration_ref("/data/config/schemas"), "declaration");
    }
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
