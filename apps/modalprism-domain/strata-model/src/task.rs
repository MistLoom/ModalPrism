//! 调度器交给 Runner 的访问任务。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::envelope::Locator;
use crate::error::ModelError;
use crate::id::{SourceId, TaskId, Timestamp};

/// 任务进入队列的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskReason {
    Schedule,
    Follow,
    RenderUpgrade,
    Detail,
    Manual,
    Replay,
}

/// 队列。各队列独立计算并发、速率和预算。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueName {
    Free,
    Paid,
    Render,
    Push,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccessTask {
    pub task_id: TaskId,
    pub source_id: SourceId,
    pub accessor: String,
    pub locator: Locator,
    pub spec: Value,
    pub reason: TaskReason,
    pub priority: i32,
    pub queue: QueueName,
    pub lease_until: Timestamp,
    pub attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_key: Option<String>,
    pub generation: u64,
}

impl AccessTask {
    pub fn validate(&self) -> Result<(), ModelError> {
        self.locator.validate()?;
        if self.accessor.is_empty() || self.batch_key.as_ref().is_some_and(|key| key.is_empty()) {
            return Err(ModelError::InvalidTask);
        }
        if !self.spec.is_object() && !self.spec.is_null() {
            return Err(ModelError::InvalidTask);
        }
        Ok(())
    }
}
