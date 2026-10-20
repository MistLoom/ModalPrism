//! 控制库上的队列和热数据。

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use strata_client::Strata;
use strata_model::{AccessTask, ApplyOpts, ChangeSet, QueueName, TaskId, Timestamp};

use crate::error::Error;

#[derive(Clone)]
pub struct Hot {
    strata: Arc<Strata>,
    kind: &'static str,
}

impl Hot {
    pub fn new(strata: Arc<Strata>, kind: &'static str) -> Self {
        Self { strata, kind }
    }
}

async fn load_bytes(hot: &Hot) -> Result<Option<Vec<u8>>, Error> {
    match hot.strata.hot_get(hot.kind, "state").await? {
        Some(value) => Ok(Some(value.to_string().into_bytes())),
        None => Ok(None),
    }
}

async fn store_bytes(hot: &Hot, bytes: &[u8]) -> Result<(), Error> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|source| Error::with_source("控制热数据读写失败", source))?;
    hot.strata.hot_put(hot.kind, "state", &value).await?;
    Ok(())
}

#[async_trait]
impl planner::Persist for Hot {
    async fn load(&self, key: &str) -> Result<Option<Vec<u8>>, planner::PlannerError> {
        if key != "state" {
            return Ok(None);
        }
        load_bytes(self)
            .await
            .map_err(|error| planner::PlannerError::with_source("控制热数据读写失败", error))
    }

    async fn store(&self, key: &str, bytes: &[u8]) -> Result<(), planner::PlannerError> {
        if key != "state" {
            return Err(planner::PlannerError::new("控制热数据读写失败"));
        }
        store_bytes(self, bytes)
            .await
            .map_err(|error| planner::PlannerError::with_source("控制热数据读写失败", error))
    }
}

#[async_trait]
impl egress::Persist for Hot {
    async fn load(&self, key: &str) -> Result<Option<Vec<u8>>, accessors::AccessError> {
        if key != "state" {
            return Ok(None);
        }
        load_bytes(self).await.map_err(|error| {
            accessors::AccessError::with_source(
                strata_model::ErrorClass::Transient,
                "控制热数据读写失败",
                error,
            )
        })
    }

    async fn store(&self, key: &str, bytes: &[u8]) -> Result<(), accessors::AccessError> {
        if key != "state" {
            return Err(accessors::AccessError::new(
                strata_model::ErrorClass::Transient,
                "控制热数据读写失败",
            ));
        }
        store_bytes(self, bytes).await.map_err(|error| {
            accessors::AccessError::with_source(
                strata_model::ErrorClass::Transient,
                "控制热数据读写失败",
                error,
            )
        })
    }
}

#[async_trait]
impl paid::Persist for Hot {
    async fn load(&self, key: &str) -> Result<Option<Vec<u8>>, accessors::AccessError> {
        egress::Persist::load(self, key).await
    }

    async fn store(&self, key: &str, bytes: &[u8]) -> Result<(), accessors::AccessError> {
        egress::Persist::store(self, key, bytes).await
    }
}

pub struct StrataQueue(pub Arc<Strata>);

fn plan_err(error: impl std::error::Error + Send + Sync + 'static) -> planner::PlannerError {
    planner::PlannerError::with_source("控制存储", error)
}

#[async_trait]
impl planner::Queue for StrataQueue {
    async fn enqueue(&self, tasks: &[AccessTask]) -> Result<(), planner::PlannerError> {
        self.0.enqueue(tasks).await.map_err(plan_err)
    }

    async fn lease_id(
        &self,
        task_id: &TaskId,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<AccessTask, planner::PlannerError> {
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
    ) -> Result<(), planner::PlannerError> {
        self.0
            .nack(task_id, retry_at, error)
            .await
            .map_err(plan_err)
    }

    async fn release_leases(&self) -> Result<Vec<AccessTask>, planner::PlannerError> {
        self.0.release_leases().await.map_err(plan_err)
    }

    async fn active(&self, source: &str, locator_key: &str) -> Result<bool, planner::PlannerError> {
        self.0
            .task_active(source, locator_key)
            .await
            .map_err(plan_err)
    }

    async fn lease_ready(
        &self,
        source: &str,
        locator_key: &str,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<Option<AccessTask>, planner::PlannerError> {
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
    ) -> Result<Vec<AccessTask>, planner::PlannerError> {
        self.0
            .lease_follows(queue, count, lease_for, now)
            .await
            .map_err(plan_err)
    }

    async fn leased_count(
        &self,
        queue: QueueName,
        now: Timestamp,
    ) -> Result<usize, planner::PlannerError> {
        self.0.leased_count(queue, now).await.map_err(plan_err)
    }

    async fn apply(&self, changeset: ChangeSet) -> Result<(), planner::PlannerError> {
        self.0
            .apply(changeset, ApplyOpts::default())
            .await
            .map_err(plan_err)?;
        Ok(())
    }

    async fn accepted_cursor(&self, source: &str) -> Result<Option<Value>, planner::PlannerError> {
        Ok(self
            .0
            .accepted_state("cursor", source)
            .await
            .map_err(plan_err)?
            .map(|state| state.value))
    }

    async fn visible_cursor(&self, source: &str) -> Result<Option<Value>, planner::PlannerError> {
        Ok(self
            .0
            .state("cursor", source)
            .await
            .map_err(plan_err)?
            .map(|state| state.value))
    }
}
