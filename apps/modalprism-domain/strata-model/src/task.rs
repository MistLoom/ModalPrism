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

impl QueueName {
    /// 队列在存储与接口上的名字。
    pub fn label(self) -> &'static str {
        match self {
            QueueName::Free => "free",
            QueueName::Paid => "paid",
            QueueName::Render => "render",
            QueueName::Push => "push",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccessTask {
    pub task_id: TaskId,
    pub source_id: SourceId,
    /// 所属采集计划。去重键带上计划维度，使「一信源多计划/多策略」互不吞并。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
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

/// 计划维度 + 定位符合成为队列去重键。计划为空时退化为纯定位符 JSON（历史兼容）。
pub fn compose_locator_key(plan_id: Option<&str>, locator: &Locator) -> String {
    let body = serde_json::to_string(locator).unwrap_or_default();
    match plan_id {
        Some(plan) if !plan.is_empty() => format!("{plan}\u{1}{body}"),
        _ => body,
    }
}

impl AccessTask {
    /// 队列去重键：同信源、同计划、同定位符视为同一活跃任务。
    pub fn locator_key(&self) -> String {
        compose_locator_key(self.plan_id.as_deref(), &self.locator)
    }

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
