//! 模型调用与通知投递的错误。区分「声明写错了」与「运行失败」，供投递映射错误码。

use std::fmt;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 声明或参数不合法，改声明即可恢复。
    #[error("{0}")]
    Invalid(String),
    /// 模型或通知端返回失败。
    #[error("{0}")]
    Remote(String),
    /// 本机文件或子进程失败。
    #[error("{0}")]
    Local(String),
}

impl Error {
    pub fn invalid(message: impl fmt::Display) -> Self {
        Self::Invalid(message.to_string())
    }

    pub fn local(message: impl fmt::Display) -> Self {
        Self::Local(message.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
