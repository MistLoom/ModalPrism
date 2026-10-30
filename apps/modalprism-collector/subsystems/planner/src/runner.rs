//! 执行已租约的任务，并把游标、回执和确认放进同一个变更集。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use strata_model::{ChangeSet, ErrorClass, Op, Producer, TaskId, Timestamp};

use crate::cadence::{CadenceInput, signal_of};
use crate::error::PlannerError;
use crate::health::{Health, Thresholds, consecutive, counts_toward_health};
use crate::persist::Persist;
use crate::planner::{
    Db, Planner, SourcePlan, SourceRow, day_of, millis, new_row, stamp, yield_sum,
};
use crate::queue::Queue;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Ok,
    Thin,
    Blocked,
    Empty,
}

#[derive(Clone, Debug)]
pub struct ReceiptBody {
    pub id: String,
    pub service: String,
    pub logical_key: String,
}

#[derive(Clone, Debug)]
pub struct TaskSuccess {
    pub ops: Vec<Op>,
    pub cursor: Option<Value>,
    pub receipt: Option<ReceiptBody>,
    pub quality: Quality,
    pub selector_miss: bool,
    pub blocked: bool,
    pub ttl: Option<Duration>,
    pub retry_after: Option<Duration>,
    pub cache_max_age: Option<Duration>,
    pub expires: Option<Duration>,
    pub new_resources: u64,
    pub render_level: Option<String>,
    pub cost: u64,
    pub adapters: Vec<String>,
    pub follows: Vec<strata_model::AccessTask>,
}

#[derive(Clone, Debug)]
pub struct Classified {
    pub class: ErrorClass,
    pub message: String,
    pub retry_after: Option<Duration>,
    pub retry_at: Option<Timestamp>,
}

#[async_trait]
pub trait Handler: Send + Sync {
    async fn execute(
        &self,
        task: &strata_model::AccessTask,
        cursor: Option<Value>,
    ) -> Result<TaskSuccess, Classified>;
}

#[async_trait]
pub trait Receipts: Send + Sync {
    async fn complete(&self, receipt_id: &str) -> Result<(), PlannerError>;
}

#[async_trait]
impl Receipts for () {
    async fn complete(&self, _receipt_id: &str) -> Result<(), PlannerError> {
        Ok(())
    }
}

impl<Q: Queue, P: Persist> Planner<Q, P> {
    pub async fn run<H: Handler, R: Receipts>(
        &self,
        plan: &SourcePlan,
        task: strata_model::AccessTask,
        handler: &H,
        receipts: &R,
        now: Timestamp,
        deadline: Timestamp,
    ) -> Result<Vec<strata_model::AccessTask>, PlannerError> {
        // 锁只保护计划状态的读改写：抓取是长 I/O，绝不能持锁执行，否则
        // `source_statuses`（信源页、内容页的项目过滤都依赖它）会排在采集之后。
        let cursor = self
            .queue()
            .accepted_cursor(plan.source_id.as_str())
            .await?;
        let cursor = if plan.migration_generation {
            // 迁移代：保留既有游标，不因 config_hash 翻转而清空回填进度（R11.3）。
            cursor
        } else {
            strip_cursor(cursor, &plan.config_hash)
        };
        let first = handler.execute(&task, cursor.clone()).await;
        let outcome = match first {
            Err(error)
                if error.class == ErrorClass::Transient
                    && now < deadline
                    && counts_toward_health(error.class, &error.message) =>
            {
                handler.execute(&task, cursor).await
            }
            other => other,
        };
        match outcome {
            Ok(success) => self.succeed(plan, &task, success, receipts, now).await,
            Err(error) => {
                self.fail(plan, &task, error, now).await?;
                Ok(Vec::new())
            }
        }
    }

    async fn succeed<R: Receipts>(
        &self,
        plan: &SourcePlan,
        task: &strata_model::AccessTask,
        success: TaskSuccess,
        receipts: &R,
        now: Timestamp,
    ) -> Result<Vec<strata_model::AccessTask>, PlannerError> {
        let _guard = self.lock().await;
        if task.reason != strata_model::TaskReason::Schedule {
            self.queue()
                .apply(changeset(plan, Some(task.task_id.clone()), success.ops))
                .await?;
            return Ok(Vec::new());
        }
        let follows = success.follows.clone();
        let mut db = self.load().await?;
        let signal = signal_of(&[
            success.ttl,
            success.retry_after,
            success.cache_max_age,
            success.expires,
        ]);
        let today = day_of(now);
        if success.new_resources > 0 {
            *db.yields
                .entry(plan.source_id.as_str().to_string())
                .or_default()
                .entry(today.clone())
                .or_insert(0) += success.new_resources;
        }
        let yield_7d = yield_sum(&db.yields, plan.source_id.as_str(), &today, 0, 7);
        let interval = next_interval_for(plan, yield_7d, signal);
        let row = db
            .sources
            .entry(plan.source_id.as_str().to_string())
            .or_insert_with(|| fresh_row(plan));
        row.interval_ms = millis(interval);
        let next = now
            .checked_add(interval)
            .map_err(|source| PlannerError::with_source("节奏", source))?;
        row.next_at = Some(stamp(next)?);
        if success.new_resources > 0 {
            row.last_yield_at = Some(stamp(now)?);
        }
        record_success(&mut db, plan, &success, now);
        let cursor = success
            .cursor
            .clone()
            .map(|value| stamp_hash(value, &plan.config_hash));
        let ops = changeset_ops(&success, cursor);
        self.save(&db).await?;
        self.queue()
            .apply(changeset(plan, Some(task.task_id.clone()), ops))
            .await?;
        if let Some(receipt) = &success.receipt {
            receipts.complete(&receipt.id).await?;
        }
        if !follows.is_empty() {
            self.queue().enqueue(&follows).await?;
        }
        Ok(follows)
    }

    async fn fail(
        &self,
        plan: &SourcePlan,
        task: &strata_model::AccessTask,
        error: Classified,
        now: Timestamp,
    ) -> Result<(), PlannerError> {
        let _guard = self.lock().await;
        tracing::warn!(
            source = plan.source_id.as_str(),
            task = task.task_id.as_str(),
            class = ?error.class,
            message = error.message,
            "任务失败"
        );
        if task.reason != strata_model::TaskReason::Schedule {
            let retry_at = error.retry_at.unwrap_or(now);
            return self
                .queue()
                .nack(&task.task_id, retry_at, &error.message)
                .await;
        }
        let mut db = self.load().await?;
        let soft = matches!(error.message.as_str(), "robots" | "unknown-receipt");
        let wait = error.message == "unknown-wait";
        let budget = error.class == ErrorClass::Budget;
        if counts_toward_health(error.class, &error.message) {
            record_failure(&mut db, plan, &error, now);
        }
        if error.class == ErrorClass::Config {
            if let Some(row) = db.sources.get_mut(plan.source_id.as_str()) {
                row.next_at = None;
            }
            self.save(&db).await?;
            return self
                .queue()
                .apply(changeset(plan, Some(task.task_id.clone()), Vec::new()))
                .await;
        }
        if soft {
            let interval = Duration::from_millis(
                db.sources
                    .get(plan.source_id.as_str())
                    .map(|row| row.interval_ms)
                    .unwrap_or_else(|| millis(plan.interval)),
            );
            if let Some(row) = db.sources.get_mut(plan.source_id.as_str()) {
                let next = now
                    .checked_add(interval)
                    .map_err(|source| PlannerError::with_source("节奏", source))?;
                row.next_at = Some(stamp(next)?);
            }
            self.save(&db).await?;
            return self
                .queue()
                .apply(changeset(plan, Some(task.task_id.clone()), Vec::new()))
                .await;
        }
        let retry_at = if budget || wait {
            error.retry_at.unwrap_or(now)
        } else if error.class == ErrorClass::Permanent {
            let ceiling = plan.ceiling;
            if let Some(row) = db.sources.get_mut(plan.source_id.as_str()) {
                row.interval_ms = millis(ceiling);
            }
            now.checked_add(Duration::from_secs(24 * 60 * 60))
                .map_err(|source| PlannerError::with_source("探测", source))?
        } else {
            let attempt = task.attempt.saturating_add(1);
            let interval = Duration::from_millis(
                db.sources
                    .get(plan.source_id.as_str())
                    .map(|row| row.interval_ms)
                    .unwrap_or_else(|| millis(plan.interval)),
            );
            let delay = backoff(interval, attempt, error.retry_after);
            now.checked_add(delay)
                .map_err(|source| PlannerError::with_source("退避", source))?
        };
        if let Some(row) = db.sources.get_mut(plan.source_id.as_str()) {
            row.next_at = Some(stamp(retry_at)?);
        }
        self.save(&db).await?;
        self.queue()
            .nack(&task.task_id, retry_at, &error.message)
            .await
    }
}

pub fn backoff(interval: Duration, attempt: u32, retry_after: Option<Duration>) -> Duration {
    let cap = Duration::from_secs(6 * 60 * 60);
    let shift = attempt.min(16);
    let scaled = interval.saturating_mul(1u32.checked_shl(shift).unwrap_or(u32::MAX));
    let bounded = scaled.min(cap);
    match retry_after {
        Some(after) => bounded.max(after),
        None => bounded,
    }
}

fn record_success(db: &mut Db, plan: &SourcePlan, success: &TaskSuccess, now: Timestamp) {
    let thresholds = thresholds(plan);
    let today = day_of(now);
    let yield_7d = yield_sum(&db.yields, plan.source_id.as_str(), &today, 0, 7);
    let prev = yield_sum(&db.yields, plan.source_id.as_str(), &today, 7, 28);
    let last_yield = db
        .sources
        .get(plan.source_id.as_str())
        .and_then(|row| row.last_yield_at.clone());
    for subject in subjects(plan, &success.adapters) {
        let health = db.health.entry(subject).or_default();
        health.transient = 0;
        health.permanent = 0;
        health.config = 0;
        health.last_ok_at = stamp(now).ok();
        health.last_error = None;
        health.alert = false;
        if let Some(level) = &success.render_level {
            health.render_level = Some(level.clone());
        }
        health.cost_7d = health.cost_7d.saturating_add(success.cost);
        if success.blocked {
            health.blocked_hits = health.blocked_hits.saturating_add(1);
        }
        health.blocked_total = health.blocked_total.saturating_add(1);
        if success.selector_miss {
            health.selector_miss = health.selector_miss.saturating_add(1);
        }
        let status = success_status(
            health,
            success,
            &thresholds,
            yield_7d,
            prev,
            last_yield.clone(),
            now,
        );
        // v3 R18：维护连续空命中轮数；产出条目即清零。
        if success.quality == Quality::Empty {
            health.empty_streak = health.empty_streak.saturating_add(1);
        } else {
            health.empty_streak = 0;
        }
        health.status = status.to_string();
        // 已判 suspect 且空命中持续到该计划类型的阈值，升级为告警（不改判定，只加强信号）。
        if status == "suspect"
            && success.quality == Quality::Empty
            && plan
                .stagnation_cycles
                .is_some_and(|cycles| health.empty_streak >= cycles)
        {
            health.alert = true;
        }
    }
}

fn success_status(
    health: &Health,
    success: &TaskSuccess,
    thresholds: &Thresholds,
    yield_7d: u64,
    prev: u64,
    last_yield: Option<String>,
    now: Timestamp,
) -> &'static str {
    if matches!(
        success.quality,
        Quality::Thin | Quality::Blocked | Quality::Empty
    ) || success.selector_miss
    {
        return "suspect";
    }
    if health.blocked_total >= 5 && health.blocked_hits * 2 >= health.blocked_total {
        return "suspect";
    }
    if let (Some(period), Some(last)) = (thresholds.expected_period_ms, last_yield) {
        if let Ok(last) = Timestamp::parse(&last) {
            if let Ok(limit) = last.checked_add(Duration::from_millis(period)) {
                if limit < now {
                    return "suspect";
                }
            }
        }
    }
    if prev > 0 {
        let mean = prev as f64 / 28.0;
        if mean > 0.0 && (yield_7d as f64 / 7.0) / mean < thresholds.yield_drop_ratio {
            return "suspect";
        }
    }
    "ok"
}

fn record_failure(db: &mut Db, plan: &SourcePlan, error: &Classified, _now: Timestamp) {
    let subject = format!("source/{}", plan.source_id);
    let (status, last_error) = {
        let health = db.health.entry(subject).or_default();
        health.last_error = Some(error.message.clone());
        match error.class {
            ErrorClass::Transient => {
                health.transient = health.transient.saturating_add(1);
                health.status = "degraded".into();
            }
            ErrorClass::Permanent => {
                health.permanent = health.permanent.saturating_add(1);
                health.status = "failing".into();
            }
            ErrorClass::Config => {
                health.config = health.config.saturating_add(1);
                health.status = "misconfigured".into();
            }
            ErrorClass::Budget => {}
        }
        health.alert = consecutive(health) >= plan.failure_threshold;
        (health.status.clone(), health.last_error.clone())
    };
    let accessor = db
        .health
        .entry(format!("accessor/{}", plan.accessor))
        .or_default();
    accessor.status = status;
    accessor.last_error = last_error;
}

fn subjects(plan: &SourcePlan, adapters: &[String]) -> Vec<String> {
    let mut out = vec![
        format!("source/{}", plan.source_id),
        format!("accessor/{}", plan.accessor),
    ];
    out.extend(adapters.iter().map(|id| format!("adapter/{id}")));
    out
}

fn thresholds(plan: &SourcePlan) -> Thresholds {
    Thresholds {
        expected_period_ms: plan.expected_period.map(millis),
        yield_drop_ratio: plan.yield_drop_ratio,
        failure_threshold: plan.failure_threshold,
    }
}

fn strip_cursor(cursor: Option<Value>, hash: &str) -> Option<Value> {
    let value = cursor?;
    if value
        .get("configHash")
        .and_then(Value::as_str)
        .is_some_and(|found| found != hash)
    {
        return None;
    }
    Some(value)
}

fn stamp_hash(value: Value, hash: &str) -> Value {
    match value {
        Value::Object(mut map) => {
            map.insert("configHash".into(), Value::String(hash.to_string()));
            Value::Object(map)
        }
        other => other,
    }
}

fn changeset_ops(success: &TaskSuccess, cursor: Option<Value>) -> Vec<Op> {
    let mut ops = success.ops.clone();
    if let Some(cursor) = cursor {
        ops.push(Op::SetState {
            scope: "cursor".into(),
            key: String::new(),
            value: cursor,
            expect_version: None,
        });
    }
    if let Some(receipt) = &success.receipt {
        ops.push(Op::SetState {
            scope: "receipt".into(),
            key: receipt.id.clone(),
            value: json!({
                "status": "completed",
                "service": receipt.service,
                "logicalKey": receipt.logical_key,
            }),
            expect_version: None,
        });
    }
    ops
}

fn changeset(plan: &SourcePlan, ack: Option<TaskId>, mut ops: Vec<Op>) -> ChangeSet {
    for op in &mut ops {
        if let Op::SetState { scope, key, .. } = op {
            if scope == "cursor" && key.is_empty() {
                *key = plan.source_id.as_str().to_string();
            }
        }
    }
    ChangeSet {
        producer: Producer {
            module: "planner".into(),
            version: "1".into(),
            config_generation: plan.generation,
        },
        idempotency_key: None,
        ack,
        ops,
    }
}

pub(crate) fn next_interval_for(plan: &SourcePlan, yield_7d: u64, signal: Duration) -> Duration {
    crate::cadence::next_interval(&CadenceInput {
        yield_7d,
        floor: plan.floor,
        ceiling: plan.ceiling,
        signal,
        paid: plan.paid,
        adaptive: plan.adaptive,
        fixed: plan.interval,
    })
}

pub(crate) fn fresh_row(plan: &SourcePlan) -> SourceRow {
    new_row(millis(plan.interval), plan.generation)
}
