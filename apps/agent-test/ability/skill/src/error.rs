use thiserror::Error;

/// Skill 域错误；保留原始语义。
#[derive(Debug, Error)]
pub enum SkillError {
    #[error("Skill 不存在: {0}")]
    NotFound(String),

    #[error("Skill 元数据无效: {0}")]
    InvalidMeta(String),

    #[error("IO 失败: {0}")]
    Io(#[from] std::io::Error),

    #[error("路径无效: {0}")]
    InvalidPath(String),
}
