use serde::{Deserialize, Serialize};

use crate::model::{Conversation, SessionItem};
use crate::run::Run;

/// 快照格式版本。更高版本拒绝导入。
pub const SNAPSHOT_FORMAT: u32 = 1;

/// 会话快照（导出 / 导入）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub format: u32,
    pub conversation: Conversation,
    #[serde(default)]
    pub items: Vec<SessionItem>,
    #[serde(default)]
    pub runs: Vec<Run>,
}

impl SessionSnapshot {
    pub fn new(conversation: Conversation, items: Vec<SessionItem>, runs: Vec<Run>) -> Self {
        Self {
            format: SNAPSHOT_FORMAT,
            conversation,
            items,
            runs,
        }
    }
}

/// 导入策略。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImportMode {
    /// 新会话 id，不导入开放回合。
    #[default]
    Copy,
    /// 保留原 id；目标已存在则冲突。
    Restore,
}
