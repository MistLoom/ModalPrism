//! 适配器错误。类别交给调度，消息保留失败原因。

use strata_model::{ErrorClass, ModelError};

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("{message}")]
    Failed {
        class: ErrorClass,
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
}

impl AdapterError {
    pub fn new(class: ErrorClass, message: impl Into<String>) -> Self {
        Self::Failed {
            class,
            message: message.into(),
            source: None,
        }
    }

    pub fn config(message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Config, message)
    }

    pub fn permanent(message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Permanent, message)
    }

    pub fn class(&self) -> ErrorClass {
        match self {
            Self::Failed { class, .. } => *class,
        }
    }
}

impl From<ModelError> for AdapterError {
    fn from(value: ModelError) -> Self {
        Self::Failed {
            class: ErrorClass::Config,
            message: value.to_string(),
            source: Some(Box::new(value)),
        }
    }
}
