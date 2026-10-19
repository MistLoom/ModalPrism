//! 契约校验错误与访问错误类别。

use serde::{Deserialize, Serialize};

/// 访问器与适配器返回的错误类别。未分类的错误按瞬时处理。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorClass {
    Transient,
    Permanent,
    Config,
    Budget,
}

impl ErrorClass {
    /// 未给出类别时按瞬时处理。
    pub fn resolve(class: Option<Self>) -> Self {
        class.unwrap_or(Self::Transient)
    }
}

/// 领域契约的校验错误。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    #[error("非法资源 ID")]
    InvalidRid,
    #[error("非法引用")]
    InvalidRef,
    #[error("非法区间")]
    InvalidSpan,
    #[error("缺少 variant")]
    MissingVariant,
    #[error("非法 variant")]
    InvalidVariant,
    #[error("非法内容哈希")]
    InvalidDigest,
    #[error("非法标识")]
    EmptyId,
    #[error("非法身份键")]
    InvalidIdentity,
    #[error("非法命名空间")]
    InvalidNamespace,
    #[error("非法时间戳")]
    InvalidTimestamp,
    #[error("非法生产者")]
    InvalidProducer,
    #[error("表达式必须是非空对象")]
    ExprNotObject,
    #[error("非法查询")]
    InvalidQuery,
    #[error("非法信封")]
    InvalidEnvelope,
    #[error("非法任务")]
    InvalidTask,
}
