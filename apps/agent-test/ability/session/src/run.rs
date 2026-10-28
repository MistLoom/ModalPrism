use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::ext::{MetricMap, RefMap};
use crate::model::{ConversationId, now_ms};

/// 回合标识（会话内一次进行中的工作单元，不绑定具体执行者类型）。
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunId(uuid::Uuid);

impl RunId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for RunId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(uuid::Uuid::parse_str(s)?))
    }
}

/// 回合状态。开放态为 Pending / Active / Suspended；其余为终态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    #[default]
    Pending,
    Active,
    Suspended,
    Completed,
    Failed,
    Cancelled,
}

impl RunStatus {
    pub fn is_open(self) -> bool {
        matches!(self, Self::Pending | Self::Active | Self::Suspended)
    }

    pub fn can_transit(self, to: Self) -> bool {
        if self == to {
            return true;
        }
        match self {
            Self::Pending => matches!(to, Self::Active | Self::Cancelled | Self::Failed),
            Self::Active => {
                matches!(
                    to,
                    Self::Suspended | Self::Completed | Self::Failed | Self::Cancelled
                )
            }
            Self::Suspended => {
                matches!(
                    to,
                    Self::Active | Self::Completed | Self::Failed | Self::Cancelled
                )
            }
            Self::Completed | Self::Failed | Self::Cancelled => false,
        }
    }
}

/// 回合。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub id: RunId,
    pub conversation_id: ConversationId,
    pub status: RunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
    #[serde(default)]
    pub refs: RefMap,
    #[serde(default)]
    pub metrics: MetricMap,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub ext: Map<String, Value>,
}

impl Run {
    pub fn new(conversation_id: ConversationId, spec: &RunSpec) -> Self {
        let now = now_ms();
        let active = spec.status == RunStatus::Active;
        Self {
            id: RunId::new(),
            conversation_id,
            status: spec.status,
            group_id: spec.group_id.clone(),
            created_at: now,
            updated_at: now,
            started_at: active.then_some(now),
            ended_at: None,
            refs: spec.refs.clone(),
            metrics: MetricMap::new(),
            checkpoint: None,
            error: None,
            ext: spec.ext.clone(),
        }
    }
}

/// 开启回合的规格。
#[derive(Clone, Debug)]
pub struct RunSpec {
    pub status: RunStatus,
    pub group_id: Option<String>,
    pub refs: RefMap,
    pub ext: Map<String, Value>,
}

impl Default for RunSpec {
    fn default() -> Self {
        Self {
            status: RunStatus::Active,
            group_id: None,
            refs: RefMap::new(),
            ext: Map::new(),
        }
    }
}

/// 回合补丁。
#[derive(Clone, Debug, Default)]
pub struct RunPatch {
    pub status: Option<RunStatus>,
    pub group_id: Option<String>,
    pub refs: Option<crate::ext::RefOp>,
    pub metrics: Option<MetricMap>,
    pub checkpoint: Option<Value>,
    pub error: Option<String>,
    pub ext: Option<Map<String, Value>>,
}

/// 回合查询。
#[derive(Clone, Debug, Default)]
pub struct RunQuery {
    pub conversation_id: Option<ConversationId>,
    pub status: Option<RunStatus>,
    pub group_id: Option<String>,
    pub limit: Option<u32>,
}
