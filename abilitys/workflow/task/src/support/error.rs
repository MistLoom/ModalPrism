//! 任务域错误；边界映射为 `ability_support::Error`，保留原文。

use thiserror::Error;

#[derive(Debug, Error)]
pub enum TaskError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Version(String),
    #[error("{0}")]
    Io(String),
}

impl From<std::io::Error> for TaskError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

impl From<serde_json::Error> for TaskError {
    fn from(value: serde_json::Error) -> Self {
        Self::Io(value.to_string())
    }
}
