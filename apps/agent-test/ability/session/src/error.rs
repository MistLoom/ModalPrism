use thiserror::Error;

/// 会话域错误；保留原始语义，禁止用通用文案覆盖。
#[derive(Debug, Error)]
pub enum SessionError {
    #[error("会话不存在: {0}")]
    NotFound(String),

    #[error("请求无效: {0}")]
    Invalid(String),

    #[error("会话冲突: {0}")]
    Conflict(String),

    #[error("存储失败: {0}")]
    Store(String),

    #[error("存储未实现: {0}")]
    Unsupported(String),
}

impl SessionError {
    pub fn not_found(id: impl ToString) -> Self {
        Self::NotFound(id.to_string())
    }

    pub fn invalid(msg: impl Into<String>) -> Self {
        Self::Invalid(msg.into())
    }

    pub fn conflict(msg: impl Into<String>) -> Self {
        Self::Conflict(msg.into())
    }

    pub fn unsupported(msg: impl Into<String>) -> Self {
        Self::Unsupported(msg.into())
    }
}
