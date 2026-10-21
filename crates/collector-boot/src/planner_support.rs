use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use planner::PlannerError;
use strata_client::Strata;
use serde_json::Value;
use strata_model::{
    AccessTask, ChangeSet, QueueName, TaskId, Timestamp,
};

pub struct StrataQueue(pub Arc<Strata>);

fn plan_err(error: strata_client::Error) -> PlannerError {
    PlannerError::with_source(error.to_string(), error)
}

#[async_trait]
impl planner::Queue for StrataQueue {
    async fn enqueue(&self, tasks: &[AccessTask]) -> Result<(), PlannerError> {
        self.0.enqueue(tasks).await.map_err(plan_err)
    }

    async fn lease_id(
        &self,
        task_id: &TaskId,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<AccessTask, PlannerError> {
        self.0
            .lease_id(task_id, lease_for, now)
            .await
            .map_err(plan_err)
    }

    async fn nack(
        &self,
        task_id: &TaskId,
        retry_at: Timestamp,
        error: &str,
    ) -> Result<(), PlannerError> {
        self.0.nack(task_id, retry_at, error).await.map_err(plan_err)
    }

    async fn release_leases(&self) -> Result<Vec<AccessTask>, PlannerError> {
        self.0.release_leases().await.map_err(plan_err)
    }

    async fn lease_ready(
        &self,
        source: &str,
        locator_key: &str,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<Option<AccessTask>, PlannerError> {
        self.0
            .lease_ready(source, locator_key, lease_for, now)
            .await
            .map_err(plan_err)
    }

    async fn lease_follows(
        &self,
        queue: QueueName,
        count: usize,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<Vec<AccessTask>, PlannerError> {
        self.0
            .lease_follows(queue, count, lease_for, now)
            .await
            .map_err(plan_err)
    }

    async fn active(&self, source: &str, locator_key: &str) -> Result<bool, PlannerError> {
        self.0
            .task_active(source, locator_key)
            .await
            .map_err(plan_err)
    }

    async fn leased_count(&self, queue: QueueName, now: Timestamp) -> Result<usize, PlannerError> {
        self.0.leased_count(queue, now).await.map_err(plan_err)
    }

    async fn apply(&self, changeset: ChangeSet) -> Result<(), PlannerError> {
        self.0
            .apply(changeset, strata_model::ApplyOpts::default())
            .await
            .map(|_| ())
            .map_err(plan_err)
    }

    async fn accepted_cursor(&self, source: &str) -> Result<Option<Value>, PlannerError> {
        Ok(self
            .0
            .accepted_state("cursor", source)
            .await
            .map_err(plan_err)?
            .map(|state| state.value))
    }

    async fn visible_cursor(&self, source: &str) -> Result<Option<Value>, PlannerError> {
        Ok(self
            .0
            .state("cursor", source)
            .await
            .map_err(plan_err)?
            .map(|state| state.value))
    }
}

pub struct PlannerPersist(pub Arc<Strata>);

#[async_trait]
impl planner::Persist for PlannerPersist {
    async fn load(&self, key: &str) -> Result<Option<Vec<u8>>, PlannerError> {
        if key != "state" {
            return Ok(None);
        }
        match self.0.hot_get("planner", "state").await.map_err(|e| PlannerError::new(e.to_string()))? {
            Some(value) => Ok(Some(value.to_string().into_bytes())),
            None => Ok(None),
        }
    }

    async fn store(&self, key: &str, bytes: &[u8]) -> Result<(), PlannerError> {
        if key != "state" {
            return Err(PlannerError::new("控制热数据读写失败"));
        }
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|e| PlannerError::new(e.to_string()))?;
        self.0
            .hot_put("planner", "state", &value)
            .await
            .map_err(|e| PlannerError::new(e.to_string()))?;
        Ok(())
    }
}
