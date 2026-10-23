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
    /// 内置扩展（sqlite-vec）注册失败。
    #[error("扩展注册失败: {0}")]
    Extension(String),
    /// 外部 RustFS 二进制：下载、校验、进程启停。
    #[error("RustFS: {0}")]
    Rustfs(String),
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
    /// 进程内 RustFS 启停或建桶失败。
    #[error("嵌入式 RustFS: {0}")]
    Embedded(String),
    #[error("暂存对象不存在: {0}")]
    MissingStaging(String),
    #[error("暂存内容与引用不一致: {0}")]
    StagingMismatch(String),
    #[error("正文与内容哈希不一致")]
    BodyMismatch,
    #[error("别名重复: {0}")]
    DuplicateAlias(String),
    #[error("别名未定义: {0}")]
    UnknownAlias(String),
    #[error("资源不存在: {0}")]
    UnknownResource(String),
    #[error("状态版本冲突: {scope}/{key} 期望 {expected} 实际 {actual}")]
    StateVersion {
        scope: String,
        key: String,
        expected: u64,
        actual: u64,
    },
    #[error("嵌入必须由派生任务写入")]
    EmbeddingUnspecified,
    #[error("外部正文必须由接入层下载")]
    ExternalUnspecified,
    #[error("标签不合法")]
    InvalidLabel,
    #[error("等待可见超时: {0}")]
    WaitTimeout(String),
    #[error("提交尚未可见: {0}")]
    NotVisible(String),
}

pub type Result<T> = std::result::Result<T, Error>;
