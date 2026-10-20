//! 访问错误。类别交给调度，消息保留调用点的原因。

use strata_model::ErrorClass;

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct AccessError {
    pub class: ErrorClass,
    pub message: String,
    #[source]
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl AccessError {
    pub fn new(class: ErrorClass, message: impl Into<String>) -> Self {
        Self {
            class,
            message: message.into(),
            source: None,
        }
    }

    pub fn with_source(
        class: ErrorClass,
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            class,
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn cancelled() -> Self {
        Self::new(ErrorClass::Transient, "cancelled")
    }
}
