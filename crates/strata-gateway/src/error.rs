//! 数据层错误。来源错误保留在 `source` 中。

use strata_model::ModelError;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Model(#[from] ModelError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("{context}: {source}")]
    Sql {
        context: &'static str,
        #[source]
        source: sqlx::Error,
    },
    /// 配置本身不成立（缺 url、空声明、阈值倒置等）。
    #[error("配置无效: {0}")]
    InvalidConfig(String),
    /// 当前编译的后端集合里没有该实现。
    #[error("{0} 后端未启用，重新编译并打开对应 feature")]
    BackendDisabled(&'static str),
    /// 该方言/索引不支持请求里用到的能力。
    #[error("不支持: {0}")]
    Unsupported(String),
    #[error("字段未定义: {0}")]
    UnknownField(String),
    #[error("过滤表达式不成立: {0}")]
    InvalidFilter(String),
    #[error("列不存在: {0}")]
    UnknownColumn(String),
    #[error("列类型不符: {0}")]
    ColumnType(String),
    #[error("快照不存在: {0}")]
    MissingSnapshot(u64),
    #[error("表版本落后于快照: {0}")]
    VersionBehind(String),
    #[error("blob 不存在: {0}")]
    MissingBlob(String),
    #[error("blob 内容与引用不一致: {0}")]
    BlobMismatch(String),
    #[error("区间越界: {sha} 请求 [{start}, {end})，实际 {len} 字节")]
    RangeOutOfBounds {
        sha: String,
        start: u64,
        end: u64,
        len: u64,
    },
    #[error("blob 阈值无效: inline={inline} dedicated={dedicated}")]
    BlobThreshold { inline: usize, dedicated: usize },
    #[error("任务不存在: {0}")]
    UnknownTask(String),
    #[error("任务状态不允许该操作: {0}")]
    TaskState(String),
    #[error("派生声明无效: {0}")]
    InvalidDerivation(String),
    #[error("派生注册表里没有: {0}")]
    UnknownDerivation(String),
    #[error("向量维度不符: 期望 {expected} 实际 {actual}")]
    VectorDims { expected: usize, actual: usize },
    #[error("向量索引不可用: {0}")]
    VectorUnavailable(String),
    #[error("全文索引不可用: {0}")]
    TextUnavailable(String),
    #[error("对象存储: {0}")]
    ObjectStore(String),
}

pub type Result<T> = std::result::Result<T, Error>;
