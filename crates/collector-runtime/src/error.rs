//! 装配错误。来源错误保留在 `source` 中。

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct Error {
    pub message: String,
    #[source]
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    pub fn with_source(
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }
}

impl From<config::Error> for Error {
    fn from(value: config::Error) -> Self {
        Self::with_source(value.to_string(), value)
    }
}

impl From<planner::PlannerError> for Error {
    fn from(value: planner::PlannerError) -> Self {
        Self::with_source(value.to_string(), value)
    }
}

impl From<strata_client::Error> for Error {
    fn from(value: strata_client::Error) -> Self {
        Self::with_source(value.to_string(), value)
    }
}

pub fn invalid_input(message: &str) -> bool {
    matches!(
        message,
        "spec 无效" | "reserve" | "gateway" | "session" | "存储位置未规定" | "监听地址不是环回地址"
    )
}
