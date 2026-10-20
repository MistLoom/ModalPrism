//! 存储错误。来源错误保留在 `source` 中。

use strata_model::ModelError;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Model(#[from] ModelError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Arrow(#[from] arrow_schema::ArrowError),
    #[error("{context}: {source}")]
    Sql {
        context: &'static str,
        #[source]
        source: sqlx::Error,
    },
    #[error("{context}: {source}")]
    Lance {
        context: String,
        #[source]
        source: lancedb::Error,
    },
    #[error("{context}: {source}")]
    LanceFormat {
        context: &'static str,
        #[source]
        source: lance::Error,
    },
    #[error("暂存文件不存在: {0}")]
    MissingStaging(String),
    #[error("暂存文件与引用不一致: {0}")]
    StagingMismatch(String),
    #[error("未定义的别名: {0}")]
    UnknownAlias(String),
    #[error("重复的别名: {0}")]
    DuplicateAlias(String),
    #[error("资源不存在: {0}")]
    UnknownResource(String),
    #[error("状态版本不符: {scope}/{key} 期望 {expected} 实际 {actual}")]
    StateVersion {
        scope: String,
        key: String,
        expected: u64,
        actual: u64,
    },
    #[error("正文与 facet 种类不匹配")]
    BodyMismatch,
    #[error("embedding 的正文形态未定义")]
    EmbeddingUnspecified,
    #[error("外部引用的内容哈希未定义")]
    ExternalUnspecified,
    #[error("标签含有 '=' 或换行")]
    InvalidLabel,
    #[error("任务不存在: {0}")]
    UnknownTask(String),
    #[error("任务已存在: {0}")]
    DuplicateTask(String),
    #[error("任务状态不允许该操作: {0}")]
    TaskState(String),
    #[error("快照不存在: {0}")]
    MissingSnapshot(u64),
    #[error("表版本落后于快照: {0}")]
    VersionBehind(String),
    #[error("快照版本与预期不一致: 预期 {expected} 实际 {actual}")]
    SnapshotVersion { expected: u64, actual: u64 },
    #[error("blobs.data 不是 Blob v2 列: {0}")]
    BlobColumn(String),
    #[error("列类型不符: {0}")]
    ColumnType(String),
    #[error("blob 不存在: {0}")]
    MissingBlob(String),
    #[error("blob 阈值无效")]
    BlobThreshold,
    #[error("等待可见超时: {0}")]
    WaitTimeout(String),
    #[error("刷写后提交仍不可见: {0}")]
    NotVisible(String),
    #[error("存在未恢复的刷写批次")]
    DirtyFlush,
    #[error("刷写记录与快照不一致")]
    PartialBatch,
}
