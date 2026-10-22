use std::fmt;

use crate::{AbilityId, InstanceId};

/// 统一错误分类：管道、端口、调度共用；消息保留原始语义。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    NotFound,
    InvalidInput,
    SessionRequired,
    SessionNotFound,
    Failed,
    Dependency,
    Config,
    Io,
    Upstream,
    Unavailable,
}

impl ErrorKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::InvalidInput => "invalid_input",
            Self::SessionRequired => "session_required",
            Self::SessionNotFound => "session_not_found",
            Self::Failed => "failed",
            Self::Dependency => "dependency_failed",
            Self::Config => "config",
            Self::Io => "io",
            Self::Upstream => "upstream",
            Self::Unavailable => "unavailable",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "not_found" => Some(Self::NotFound),
            "invalid_input" => Some(Self::InvalidInput),
            "session_required" => Some(Self::SessionRequired),
            "session_not_found" => Some(Self::SessionNotFound),
            "failed" => Some(Self::Failed),
            "dependency_failed" => Some(Self::Dependency),
            "config" => Some(Self::Config),
            "io" => Some(Self::Io),
            "upstream" => Some(Self::Upstream),
            "unavailable" => Some(Self::Unavailable),
            _ => None,
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// 框架统一错误载体。禁止泛化覆盖原始消息。
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    message: String,
    ability: Option<AbilityId>,
    instance: Option<InstanceId>,
    cause: Option<Box<Error>>,
}

impl Error {
    pub fn new(kind: ErrorKind, message: impl AsRef<str>) -> Self {
        Self {
            kind,
            message: message.as_ref().to_string(),
            ability: None,
            instance: None,
            cause: None,
        }
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn code(&self) -> &'static str {
        self.kind.code()
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn ability_id(&self) -> Option<&AbilityId> {
        self.ability.as_ref()
    }

    pub fn instance_id(&self) -> Option<&InstanceId> {
        self.instance.as_ref()
    }

    pub fn with_ability(mut self, id: AbilityId) -> Self {
        self.ability = Some(id);
        self
    }

    pub fn with_instance(mut self, id: InstanceId) -> Self {
        self.instance = Some(id);
        self
    }

    pub fn not_found(id: impl fmt::Display) -> Self {
        Self::new(ErrorKind::NotFound, format!("不存在: {id}"))
    }

    pub fn invalid_args(message: impl AsRef<str>) -> Self {
        Self::new(ErrorKind::InvalidInput, message)
    }

    pub fn session_required() -> Self {
        Self::new(ErrorKind::SessionRequired, "有状态调用需要 locals 作用域")
    }

    pub fn session_not_found(id: impl AsRef<str>) -> Self {
        Self::new(
            ErrorKind::SessionNotFound,
            format!("locals 作用域不存在: {}", id.as_ref()),
        )
    }

    pub fn failed(message: impl AsRef<str>) -> Self {
        Self::new(ErrorKind::Failed, message)
    }

    pub fn dependency(id: impl fmt::Display, source: Error) -> Self {
        let message = format!("依赖调用失败 ({id}): {source}");
        Self {
            kind: ErrorKind::Dependency,
            message,
            ability: None,
            instance: None,
            cause: Some(Box::new(source)),
        }
    }

    pub fn config(message: impl AsRef<str>) -> Self {
        Self::new(ErrorKind::Config, message)
    }

    pub fn io(message: impl AsRef<str>) -> Self {
        Self::new(ErrorKind::Io, message)
    }

    pub fn upstream(status: u16, body: impl AsRef<str>) -> Self {
        Self::new(
            ErrorKind::Upstream,
            format!("上游返回错误 (status={status}): {}", body.as_ref()),
        )
    }

    pub fn unavailable(message: impl AsRef<str>) -> Self {
        Self::new(ErrorKind::Unavailable, message)
    }

    pub fn from_ext(kind: ErrorKind, err: impl fmt::Display) -> Self {
        Self::new(kind, err.to_string())
    }

    /// Undo 失败挂到原始错误的 cause，不覆盖原文。
    pub fn with_undo_error(mut self, undo_err: Error) -> Self {
        let mut chain = &mut self;
        while chain.cause.is_some() {
            chain = chain.cause.as_mut().unwrap();
        }
        chain.cause = Some(Box::new(Error::new(
            ErrorKind::Failed,
            format!("Undo 失败: {undo_err}"),
        )));
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause
            .as_ref()
            .map(|e| e as &(dyn std::error::Error + 'static))
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::io(value.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
