use thiserror::Error;

/// 协议层错误；保留原始语义，禁止泛化覆盖。
#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("请求无效: {0}")]
    InvalidRequest(String),

    #[error("序列化失败: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("传输失败: {0}")]
    Transport(String),

    #[error("上游返回错误 (status={status}): {body}")]
    Upstream { status: u16, body: String },

    #[error("协议能力未启用: {0}")]
    Unsupported(String),
}
