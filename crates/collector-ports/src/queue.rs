use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use planner::PlannerError;
use serde_json::{Value, json};
use strata_model::{AccessTask, ChangeSet, QueueName, TaskId, Timestamp};

use crate::call::CtxCall;

/// 队列操作全部经 `strata_queue` Ability。`apply` 可选走 `strata_write`。
pub struct CtxQueue {
    ctx: Arc<dyn CtxCall>,
    apply_via_write: bool,
}

impl CtxQueue {
    pub fn new(ctx: Arc<dyn CtxCall>) -> Self {
        Self {
            ctx,
            apply_via_write: false,
        }
    }

    pub fn runner(ctx: Arc<dyn CtxCall>) -> Self {
        Self {
            ctx,
            apply_via_write: true,
        }
    }

    async fn call(&self, method: &str, args: Value) -> Result<Value, PlannerError> {
        self.ctx.call("strata_queue", method, args).await
    }

    fn plan_err(message: &str) -> PlannerError {
        PlannerError::new(message)
    }
}

#[async_trait]
impl planner::Queue for CtxQueue {
    async fn enqueue(&self, tasks: &[AccessTask]) -> Result<(), PlannerError> {
        self.call("enqueue", json!({ "tasks": tasks }))
            .await?;
        Ok(())
    }

    async fn lease_id(
        &self,
        task_id: &TaskId,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<AccessTask, PlannerError> {
        let value = self
            .call(
                "lease_id",
                json!({
                    "taskId": task_id.to_string(),
                    "leaseMs": lease_for.as_millis(),
                    "now": now.to_string(),
                }),
            )
            .await?;
        serde_json::from_value(value).map_err(|_| Self::plan_err("租约"))
    }

    async fn nack(
        &self,
        task_id: &TaskId,
        retry_at: Timestamp,
        error: &str,
    ) -> Result<(), PlannerError> {
        self.call(
            "nack",
            json!({
                "taskId": task_id.to_string(),
                "retryAt": retry_at.to_string(),
                "error": error,
            }),
        )
        .await?;
        Ok(())
    }

    async fn release_leases(&self) -> Result<Vec<AccessTask>, PlannerError> {
        let value = self.call("release_leases", json!({})).await?;
        serde_json::from_value(value).map_err(|_| Self::plan_err("租约"))
    }

    async fn release_source_leases(&self, source: &str) -> Result<(), PlannerError> {
        self.call("release_source_leases", json!({ "source": source }))
            .await?;
        Ok(())
    }

    async fn active(&self, source: &str, locator_key: &str) -> Result<bool, PlannerError> {
        let value = self
            .call(
                "active",
                json!({ "source": source, "locatorKey": locator_key }),
            )
            .await?;
        value
            .as_bool()
            .ok_or_else(|| Self::plan_err("活跃"))
    }

    async fn lease_ready(
        &self,
        source: &str,
        locator_key: &str,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<Option<AccessTask>, PlannerError> {
        let value = self
            .call(
                "lease_ready",
                json!({
                    "source": source,
                    "locatorKey": locator_key,
                    "leaseMs": lease_for.as_millis(),
                    "now": now.to_string(),
                }),
            )
            .await?;
        if value.is_null() {
            Ok(None)
        } else {
            serde_json::from_value(value).map(Some).map_err(|_| Self::plan_err("租约"))
        }
    }

    async fn lease_follows(
        &self,
        queue: QueueName,
        count: usize,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<Vec<AccessTask>, PlannerError> {
        let value = self
            .call(
                "lease_follows",
                json!({
                    "queue": queue,
                    "count": count,
                    "leaseMs": lease_for.as_millis(),
                    "now": now.to_string(),
                }),
            )
            .await?;
        serde_json::from_value(value).map_err(|_| Self::plan_err("租约"))
    }

    async fn leased_count(&self, queue: QueueName, now: Timestamp) -> Result<usize, PlannerError> {
        let value = self
            .call(
                "leased_count",
                json!({ "queue": queue, "now": now.to_string() }),
            )
            .await?;
        value
            .as_u64()
            .map(|n| n as usize)
            .ok_or_else(|| Self::plan_err("计数"))
    }

    async fn apply(&self, changeset: ChangeSet) -> Result<(), PlannerError> {
        if self.apply_via_write && changeset.ack.is_none() && changeset.ops.is_empty() {
            return Ok(());
        }
        if self.apply_via_write && changeset.ack.is_none() {
            self.ctx
                .call("strata_write", "apply", json!({ "changeset": changeset }))
                .await?;
            return Ok(());
        }
        self.call("apply", json!({ "changeset": changeset }))
            .await?;
        Ok(())
    }

    async fn accepted_cursor(&self, source: &str) -> Result<Option<Value>, PlannerError> {
        let value = self
            .call("accepted_cursor", json!({ "source": source }))
            .await?;
        if value.is_null() {
            Ok(None)
        } else {
            Ok(Some(value))
        }
    }

    async fn visible_cursor(&self, source: &str) -> Result<Option<Value>, PlannerError> {
        let value = self
            .call("visible_cursor", json!({ "source": source }))
            .await?;
        if value.is_null() {
            Ok(None)
        } else {
            Ok(Some(value))
        }
    }
}
