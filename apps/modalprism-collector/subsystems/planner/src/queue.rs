//! 队列端口。控制库的 `tasks` 表是这个端口的存储实现。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use strata_model::{AccessTask, ChangeSet, QueueName, TaskId, Timestamp};
use tokio::sync::Mutex;

use crate::error::PlannerError;

#[async_trait]
pub trait Queue: Send + Sync {
    async fn enqueue(&self, tasks: &[AccessTask]) -> Result<(), PlannerError>;
    async fn lease_id(
        &self,
        task_id: &TaskId,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<AccessTask, PlannerError>;
    async fn nack(
        &self,
        task_id: &TaskId,
        retry_at: Timestamp,
        error: &str,
    ) -> Result<(), PlannerError>;
    async fn release_leases(&self) -> Result<Vec<AccessTask>, PlannerError>;
    async fn active(&self, source: &str, locator_key: &str) -> Result<bool, PlannerError>;
    async fn lease_ready(
        &self,
        source: &str,
        locator_key: &str,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<Option<AccessTask>, PlannerError>;
    async fn lease_follows(
        &self,
        queue: QueueName,
        count: usize,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<Vec<AccessTask>, PlannerError>;
    async fn leased_count(&self, queue: QueueName, now: Timestamp) -> Result<usize, PlannerError>;
    async fn apply(&self, changeset: ChangeSet) -> Result<(), PlannerError>;
    async fn accepted_cursor(&self, source: &str) -> Result<Option<Value>, PlannerError>;
    async fn visible_cursor(&self, source: &str) -> Result<Option<Value>, PlannerError>;
}

#[derive(Clone)]
struct Row {
    task: AccessTask,
    status: String,
    retry_at: Option<Timestamp>,
    locator_key: String,
}

#[derive(Default)]
pub struct MemoryQueue {
    rows: Mutex<Vec<Row>>,
    accepted: Mutex<Vec<(String, String, Value)>>,
    visible: Mutex<Vec<(String, String, Value)>>,
}

impl MemoryQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn publish(&self) {
        *self.visible.lock().await = self.accepted.lock().await.clone();
    }
}

#[async_trait]
impl Queue for MemoryQueue {
    async fn enqueue(&self, tasks: &[AccessTask]) -> Result<(), PlannerError> {
        let mut rows = self.rows.lock().await;
        for task in tasks {
            task.validate()
                .map_err(|source| PlannerError::with_source("任务无效", source))?;
            if rows.iter().any(|row| row.task.task_id == task.task_id) {
                return Err(PlannerError::new(format!("任务已存在: {}", task.task_id)));
            }
            let locator_key = serde_json::to_string(&task.locator)
                .map_err(|source| PlannerError::with_source("定位符", source))?;
            if rows.iter().any(|row| {
                row.status != "done"
                    && row.task.source_id.as_str() == task.source_id.as_str()
                    && row.locator_key == locator_key
            }) {
                continue;
            }
            rows.push(Row {
                task: task.clone(),
                status: "queued".into(),
                retry_at: None,
                locator_key,
            });
        }
        Ok(())
    }

    async fn lease_id(
        &self,
        task_id: &TaskId,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<AccessTask, PlannerError> {
        let mut rows = self.rows.lock().await;
        let row = rows
            .iter_mut()
            .find(|row| row.task.task_id == *task_id)
            .ok_or_else(|| PlannerError::new("任务不存在"))?;
        if row.status == "done" {
            return Err(PlannerError::new("任务状态不允许该操作"));
        }
        if row.status == "leased" && row.task.lease_until > now {
            return Err(PlannerError::new("任务状态不允许该操作"));
        }
        if row.retry_at.is_some_and(|retry| retry > now) {
            return Err(PlannerError::new("任务状态不允许该操作"));
        }
        let until = now
            .checked_add(lease_for)
            .map_err(|source| PlannerError::with_source("租约时间", source))?;
        row.task.lease_until = until;
        row.status = "leased".into();
        Ok(row.task.clone())
    }

    async fn nack(
        &self,
        task_id: &TaskId,
        retry_at: Timestamp,
        error: &str,
    ) -> Result<(), PlannerError> {
        let mut rows = self.rows.lock().await;
        let row = rows
            .iter_mut()
            .find(|row| row.task.task_id == *task_id)
            .ok_or_else(|| PlannerError::new("任务不存在"))?;
        if row.status == "done" {
            return Err(PlannerError::new("任务状态不允许该操作"));
        }
        row.task.attempt = row.task.attempt.saturating_add(1);
        row.status = "queued".into();
        row.retry_at = Some(retry_at);
        let _ = error;
        Ok(())
    }

    async fn release_leases(&self) -> Result<Vec<AccessTask>, PlannerError> {
        let mut rows = self.rows.lock().await;
        let mut released = Vec::new();
        let released_at = Timestamp::now();
        for row in rows.iter_mut().filter(|row| row.status == "leased") {
            released.push(row.task.clone());
            row.status = "queued".into();
            row.task.lease_until = released_at;
        }
        Ok(released)
    }

    async fn active(&self, source: &str, locator_key: &str) -> Result<bool, PlannerError> {
        let rows = self.rows.lock().await;
        Ok(rows.iter().any(|row| {
            row.status != "done"
                && row.task.source_id.as_str() == source
                && row.locator_key == locator_key
        }))
    }

    async fn lease_ready(
        &self,
        source: &str,
        locator_key: &str,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<Option<AccessTask>, PlannerError> {
        let ready = {
            let rows = self.rows.lock().await;
            let Some(row) = rows.iter().find(|row| {
                row.status != "done"
                    && row.task.source_id.as_str() == source
                    && row.locator_key == locator_key
            }) else {
                return Ok(None);
            };
            let live = row.status == "leased" && row.task.lease_until > now;
            let waiting = row.retry_at.is_some_and(|retry| retry > now);
            if live || waiting {
                None
            } else {
                Some(row.task.task_id.clone())
            }
        };
        let Some(task_id) = ready else {
            return Ok(None);
        };
        self.lease_id(&task_id, lease_for, now).await.map(Some)
    }

    async fn lease_follows(
        &self,
        queue: QueueName,
        count: usize,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<Vec<AccessTask>, PlannerError> {
        if count == 0 {
            return Ok(Vec::new());
        }
        let ids = {
            let rows = self.rows.lock().await;
            rows.iter()
                .filter(|row| {
                    row.task.queue == queue
                        && row.task.reason == strata_model::TaskReason::Follow
                        && row.status != "done"
                        && !(row.status == "leased" && row.task.lease_until > now)
                        && row.retry_at.is_none_or(|retry| retry <= now)
                })
                .take(count)
                .map(|row| row.task.task_id.clone())
                .collect::<Vec<_>>()
        };
        let mut leased = Vec::with_capacity(ids.len());
        for task_id in ids {
            leased.push(self.lease_id(&task_id, lease_for, now).await?);
        }
        Ok(leased)
    }

    async fn leased_count(&self, queue: QueueName, now: Timestamp) -> Result<usize, PlannerError> {
        let rows = self.rows.lock().await;
        Ok(rows
            .iter()
            .filter(|row| {
                row.task.queue == queue && row.status == "leased" && row.task.lease_until > now
            })
            .count())
    }

    async fn apply(&self, changeset: ChangeSet) -> Result<(), PlannerError> {
        if let Some(task_id) = &changeset.ack {
            let mut rows = self.rows.lock().await;
            let row = rows
                .iter_mut()
                .find(|row| row.task.task_id == *task_id)
                .ok_or_else(|| PlannerError::new("任务不存在"))?;
            if row.status == "done" {
                return Err(PlannerError::new("任务状态不允许该操作"));
            }
            row.status = "done".into();
        }
        let mut accepted = self.accepted.lock().await;
        for op in &changeset.ops {
            if let strata_model::Op::SetState {
                scope, key, value, ..
            } = op
            {
                accepted
                    .retain(|(found_scope, found_key, _)| found_scope != scope || found_key != key);
                accepted.push((scope.clone(), key.clone(), value.clone()));
            }
        }
        Ok(())
    }

    async fn accepted_cursor(&self, source: &str) -> Result<Option<Value>, PlannerError> {
        Ok(self
            .accepted
            .lock()
            .await
            .iter()
            .rev()
            .find(|(scope, key, _)| scope == "cursor" && key == source)
            .map(|(_, _, value)| value.clone()))
    }

    async fn visible_cursor(&self, source: &str) -> Result<Option<Value>, PlannerError> {
        Ok(self
            .visible
            .lock()
            .await
            .iter()
            .rev()
            .find(|(scope, key, _)| scope == "cursor" && key == source)
            .map(|(_, _, value)| value.clone()))
    }
}
