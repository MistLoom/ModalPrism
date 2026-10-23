//! 派生任务的执行方：租约 → 读输入 → 调产出方 → 回写派生 facet 并在同一变更集确认。
//!
//! 数据层只提供载荷与血缘字段；模型调用、OCR 与区域切分由实现 `DerivationProducer`
//! 的模块提供。执行方只领取 `reason = detail` 的任务，不会碰信源调度与跟进任务。

use std::time::Duration;

use async_trait::async_trait;
use strata_gateway::DerivationTask;
use strata_model::{
    AccessTask, ApplyOpts, Body, ChangeSet, FacetKind, FacetRef, LineageInput, Op, QueueName, Rid,
    SpanRef, Target, Timestamp,
};

use crate::{Error, Facet, Strata};

/// 一次派生任务的输入。
#[derive(Clone, Copy, Debug)]
pub struct DerivationInput<'a> {
    pub task: &'a DerivationTask,
    /// 按任务钉住的版本读到的输入 facet。
    pub input: &'a Facet,
}

/// 产出方。返回 `None` 表示这次不产出；任务会被确认，下一次发布时
/// 再按快照重新评估。
#[async_trait]
pub trait DerivationProducer: Send + Sync {
    async fn produce(&self, input: DerivationInput<'_>) -> Result<Option<Body>, String>;
}

/// 单条任务的执行结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DerivationStep {
    /// 写入了派生 facet。
    Produced,
    /// 输入不存在或产出方放弃，任务已确认。
    Skipped,
    /// 产出方报错，任务已退回队列。
    Failed,
}

/// 一批派生任务的执行统计。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DerivationRun {
    pub leased: usize,
    pub produced: usize,
    pub skipped: usize,
    pub failed: usize,
}

impl Strata {
    /// 领取派生任务。只取 `reason = detail` 的任务。
    pub async fn lease_derivations(
        &self,
        queue: QueueName,
        count: usize,
        lease_for: Duration,
    ) -> Result<Vec<AccessTask>, Error> {
        Ok(self
            .gateway
            .lease_derivations(queue, count, lease_for, Timestamp::now())
            .await?)
    }

    /// 执行一条已租约的派生任务。
    pub async fn run_derivation<P: DerivationProducer>(
        &self,
        producer: &P,
        task: &AccessTask,
    ) -> Result<DerivationStep, Error> {
        let payload = self.gateway.derivation(task)?;
        let input_ref = FacetRef::parse(&payload.input)?;
        let Some(input) = self.get(&input_ref, false).await? else {
            // 输入 facet 已经读不到，再排队也不会出现；确认掉，避免毒消息。
            self.gateway.ack(&task.task_id).await?;
            return Ok(DerivationStep::Skipped);
        };
        let body = match producer
            .produce(DerivationInput {
                task: &payload,
                input: &input,
            })
            .await
        {
            Ok(Some(body)) => body,
            Ok(None) => {
                self.gateway.ack(&task.task_id).await?;
                return Ok(DerivationStep::Skipped);
            }
            Err(message) => {
                let retry = Timestamp::now()
                    .checked_add(Duration::from_secs(60))
                    .unwrap_or_else(|_| Timestamp::now());
                self.gateway.nack(&task.task_id, retry, &message).await?;
                return Ok(DerivationStep::Failed);
            }
        };
        let span = SpanRef::new(input_ref, 0, payload.input_size)?;
        let op = Op::PutFacet {
            target: Target::Rid(Rid::parse(&payload.rid)?),
            kind: FacetKind::parse(&payload.produces)?,
            variant: payload.produces_variant.clone(),
            body,
            media_type: None,
            lineage: LineageInput {
                inputs: vec![span],
                params_hash: payload.params_hash.clone(),
            },
        };
        let changeset = ChangeSet {
            producer: payload.producer.clone(),
            // 任务号做幂等键：崩溃重试不会写出第二份派生 facet。
            idempotency_key: Some(task.task_id.as_str().to_string()),
            ack: Some(task.task_id.clone()),
            ops: vec![op],
        };
        self.apply(changeset, ApplyOpts::default()).await?;
        Ok(DerivationStep::Produced)
    }

    /// 领取并执行一批派生任务，有写入时统一 `flush` 一次。
    pub async fn run_derivations<P: DerivationProducer>(
        &self,
        producer: &P,
        queue: QueueName,
        count: usize,
        lease_for: Duration,
    ) -> Result<DerivationRun, Error> {
        let tasks = self.lease_derivations(queue, count, lease_for).await?;
        let mut run = DerivationRun {
            leased: tasks.len(),
            ..DerivationRun::default()
        };
        for task in &tasks {
            match self.run_derivation(producer, task).await? {
                DerivationStep::Produced => run.produced += 1,
                DerivationStep::Skipped => run.skipped += 1,
                DerivationStep::Failed => run.failed += 1,
            }
        }
        if run.produced > 0 {
            self.flush().await?;
        }
        Ok(run)
    }
}
