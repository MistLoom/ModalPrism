//! 到期扫描、成组入队和租约。

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use strata_model::{AccessTask, Locator, Op, QueueName, SourceId, TaskId, TaskReason, Timestamp};
use tokio::sync::Mutex;

use crate::cadence::{CadenceInput, next_interval};
use crate::error::PlannerError;
use crate::health::Health;
use crate::persist::Persist;
use crate::queue::Queue;

#[derive(Clone, Debug)]
pub struct SourcePlan {
    pub source_id: SourceId,
    pub locator: Locator,
    pub accessor: String,
    pub queue: QueueName,
    pub spec: Value,
    pub generation: u64,
    pub config_hash: String,
    pub timeout: Duration,
    pub paid: bool,
    pub budget_requests: Option<u64>,
    pub budget_window: Duration,
    pub adaptive: bool,
    pub interval: Duration,
    pub floor: Duration,
    pub ceiling: Duration,
    pub batch_key: Option<String>,
    pub enabled: bool,
    pub expected_period: Option<Duration>,
    pub yield_drop_ratio: f64,
    pub failure_threshold: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceStatus {
    pub next_at: Option<String>,
    pub interval_ms: u64,
    pub health_status: String,
    pub alert: bool,
    pub skip_reason: Option<String>,
    pub budget_used: u64,
    pub last_error: Option<String>,
    pub last_ok_at: Option<String>,
    pub last_yield_at: Option<String>,
    pub yields_7d: u64,
}

pub struct Capacities {
    pub free: usize,
    pub paid: usize,
    pub render: usize,
    pub push: usize,
}

impl Capacities {
    fn room(&self, queue: QueueName) -> usize {
        match queue {
            QueueName::Free => self.free,
            QueueName::Paid => self.paid,
            QueueName::Render => self.render,
            QueueName::Push => self.push,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct Db {
    pub sources: BTreeMap<String, SourceRow>,
    pub health: BTreeMap<String, Health>,
    pub yields: BTreeMap<String, BTreeMap<String, u64>>,
    pub last_snapshot: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SourceRow {
    pub next_at: Option<String>,
    pub interval_ms: u64,
    pub generation: u64,
    pub last_yield_at: Option<String>,
    pub last_recompute: Option<String>,
    #[serde(default)]
    pub paid_window_start: i64,
    #[serde(default)]
    pub paid_used: u64,
    #[serde(default)]
    pub skip_reason: Option<String>,
}

pub struct Planner<Q, P> {
    queue: Q,
    persist: P,
    lock: Mutex<()>,
}

impl<Q: Queue, P: Persist> Planner<Q, P> {
    pub fn new(queue: Q, persist: P) -> Self {
        Self {
            queue,
            persist,
            lock: Mutex::new(()),
        }
    }

    pub fn queue(&self) -> &Q {
        &self.queue
    }

    pub async fn tick(
        &self,
        plans: &[SourcePlan],
        capacities: &Capacities,
        now: Timestamp,
    ) -> Result<Vec<AccessTask>, PlannerError> {
        let _guard = self.lock.lock().await;
        let mut db = load_db(&self.persist).await?;
        let today = day_of(now);
        for plan in plans {
            prepare_source(&mut db, plan, &today, now);
        }
        let due = due_ids(&db, plans, now);
        let mut chosen = Vec::new();
        for plan in plans {
            if !due.contains(plan.source_id.as_str()) || !plan.enabled {
                continue;
            }
            chosen.push(plan);
        }
        chosen.sort_by(|left, right| {
            let left_key = dispatch_key(&db, left);
            let right_key = dispatch_key(&db, right);
            right_key
                .0
                .cmp(&left_key.0)
                .then_with(|| left_key.1.cmp(&right_key.1))
        });
        let mut tasks = Vec::new();
        let mut used = [0usize; 4];
        for plan in chosen {
            let slot = queue_slot(plan.queue);
            let occupied = self.queue.leased_count(plan.queue, now).await?;
            if occupied.saturating_add(used[slot]) >= capacities.room(plan.queue) {
                continue;
            }
            let lease_for = lease_span(plan.timeout);
            let key = locator_key(&plan.locator)?;
            let leased = if self.queue.active(plan.source_id.as_str(), &key).await? {
                let Some(task) = self
                    .queue
                    .lease_ready(plan.source_id.as_str(), &key, lease_for, now)
                    .await?
                else {
                    continue;
                };
                task
            } else if charge_budget(&mut db, plan, now)? {
                let task = make_task(plan, now)?;
                self.queue.enqueue(std::slice::from_ref(&task)).await?;
                self.queue.lease_id(&task.task_id, lease_for, now).await?
            } else {
                continue;
            };
            used[slot] += 1;
            if let Some(row) = db.sources.get_mut(plan.source_id.as_str()) {
                let until = now
                    .checked_add(lease_for)
                    .map_err(|source| PlannerError::with_source("租约时间", source))?;
                row.next_at = Some(stamp(until)?);
            }
            tasks.push(leased);
        }
        let follow_occupied = self.queue.leased_count(QueueName::Free, now).await?;
        let follow_room = capacities
            .room(QueueName::Free)
            .saturating_sub(follow_occupied);
        if follow_room > 0 {
            let lease_for = plans
                .iter()
                .filter(|plan| plan.enabled && plan.queue == QueueName::Free)
                .map(|plan| lease_span(plan.timeout))
                .max()
                .unwrap_or_else(|| lease_span(Duration::from_secs(45)));
            let follows = self
                .queue
                .lease_follows(QueueName::Free, follow_room, lease_for, now)
                .await?;
            tasks.extend(follows);
        }
        save_db(&self.persist, &db).await?;
        Ok(tasks)
    }

    pub async fn source_status(&self, source: &str) -> Result<SourceStatus, PlannerError> {
        let _guard = self.lock.lock().await;
        let db = load_db(&self.persist).await?;
        Ok(status_of(&db, source, Timestamp::now()))
    }

    /// 所有已有调度状态的信号源，按 id 排序。
    pub async fn source_statuses(&self) -> Result<Vec<(String, SourceStatus)>, PlannerError> {
        let _guard = self.lock.lock().await;
        let db = load_db(&self.persist).await?;
        let now = Timestamp::now();
        Ok(db
            .sources
            .keys()
            .map(|source| (source.clone(), status_of(&db, source, now)))
            .collect())
    }

    pub async fn make_due(&self, source: &str, now: Timestamp) -> Result<(), PlannerError> {
        if source.is_empty() {
            return Err(PlannerError::new("信源为空"));
        }
        let _guard = self.lock.lock().await;
        let mut db = load_db(&self.persist).await?;
        let row = db
            .sources
            .entry(source.to_string())
            .or_insert_with(|| new_row(0, 0));
        row.next_at = Some(stamp(now)?);
        save_db(&self.persist, &db).await
    }

    pub async fn shutdown(&self, now: Timestamp) -> Result<Vec<AccessTask>, PlannerError> {
        let _guard = self.lock.lock().await;
        let released = self.queue.release_leases().await?;
        let mut db = load_db(&self.persist).await?;
        for task in &released {
            if let Some(row) = db.sources.get_mut(task.source_id.as_str()) {
                row.next_at = Some(stamp(now)?);
            }
        }
        save_db(&self.persist, &db).await?;
        Ok(released)
    }

    pub async fn note_yield(
        &self,
        source: &str,
        now: Timestamp,
        count: u64,
    ) -> Result<(), PlannerError> {
        let _guard = self.lock.lock().await;
        let mut db = load_db(&self.persist).await?;
        *db.yields
            .entry(source.to_string())
            .or_default()
            .entry(day_of(now))
            .or_insert(0) += count;
        if let Some(row) = db.sources.get_mut(source) {
            row.last_yield_at = Some(stamp(now)?);
        }
        save_db(&self.persist, &db).await
    }

    pub async fn snapshot(
        &self,
        now: Timestamp,
        interval: Duration,
    ) -> Result<Vec<Op>, PlannerError> {
        let _guard = self.lock.lock().await;
        let db = load_db(&self.persist).await?;
        if let Some(last) = db.last_snapshot.as_deref() {
            let last = Timestamp::parse(last)
                .map_err(|source| PlannerError::with_source("快照时间", source))?;
            if let Ok(next) = last.checked_add(interval) {
                if next > now {
                    return Ok(Vec::new());
                }
            }
        }
        let mut ops = Vec::new();
        for (key, health) in &db.health {
            ops.push(Op::SetState {
                scope: "health".into(),
                key: key.clone(),
                value: serde_json::to_value(health)
                    .map_err(|source| PlannerError::with_source("健康", source))?,
                expect_version: None,
            });
        }
        for (key, row) in &db.sources {
            ops.push(Op::SetState {
                scope: "cadence".into(),
                key: key.clone(),
                value: json!({
                    "nextAt": row.next_at,
                    "intervalMs": row.interval_ms,
                }),
                expect_version: None,
            });
        }
        Ok(ops)
    }

    pub async fn commit_snapshot(&self, now: Timestamp) -> Result<(), PlannerError> {
        let _guard = self.lock.lock().await;
        let mut db = load_db(&self.persist).await?;
        db.last_snapshot = Some(stamp(now)?);
        save_db(&self.persist, &db).await
    }

    pub(crate) async fn load(&self) -> Result<Db, PlannerError> {
        load_db(&self.persist).await
    }

    pub(crate) async fn save(&self, db: &Db) -> Result<(), PlannerError> {
        save_db(&self.persist, db).await
    }

    pub(crate) async fn lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.lock.lock().await
    }
}

/// 新的计划任务消耗一次窗口内的付费请求。次数用尽则记 `budget`，并把下次时间推到下一窗口。
fn charge_budget(db: &mut Db, plan: &SourcePlan, now: Timestamp) -> Result<bool, PlannerError> {
    let Some(limit) = plan.budget_requests else {
        return Ok(true);
    };
    let window_ms = i64::try_from(plan.budget_window.as_millis()).unwrap_or(0);
    if window_ms <= 0 {
        return Err(PlannerError::new("预算窗口为空"));
    }
    let now_ms = now
        .unix_millis()
        .map_err(|source| PlannerError::with_source("预算窗口", source))?;
    let start = now_ms.div_euclid(window_ms) * window_ms;
    let row = db
        .sources
        .get_mut(plan.source_id.as_str())
        .ok_or_else(|| PlannerError::new("信源缺少节奏"))?;
    if row.paid_window_start != start {
        row.paid_window_start = start;
        row.paid_used = 0;
        row.skip_reason = None;
    }
    if row.paid_used >= limit {
        row.skip_reason = Some("budget".into());
        let end = start.saturating_add(window_ms);
        let until = Timestamp::from_unix_millis(end)
            .map_err(|source| PlannerError::with_source("预算窗口", source))?;
        row.next_at = Some(stamp(until)?);
        return Ok(false);
    }
    row.paid_used = row.paid_used.saturating_add(1);
    row.skip_reason = None;
    Ok(true)
}

pub(crate) fn new_row(interval_ms: u64, generation: u64) -> SourceRow {
    SourceRow {
        next_at: None,
        interval_ms,
        generation,
        last_yield_at: None,
        last_recompute: None,
        paid_window_start: 0,
        paid_used: 0,
        skip_reason: None,
    }
}

fn dispatch_key(db: &Db, plan: &SourcePlan) -> (i32, String) {
    let at = db
        .sources
        .get(plan.source_id.as_str())
        .and_then(|row| row.next_at.clone())
        .unwrap_or_default();
    (priority(TaskReason::Schedule), at)
}

fn queue_slot(queue: QueueName) -> usize {
    match queue {
        QueueName::Free => 0,
        QueueName::Paid => 1,
        QueueName::Render => 2,
        QueueName::Push => 3,
    }
}

pub fn priority(reason: TaskReason) -> i32 {
    match reason {
        TaskReason::Manual | TaskReason::Replay => 30,
        TaskReason::Follow | TaskReason::Detail | TaskReason::RenderUpgrade => 20,
        TaskReason::Schedule => 10,
    }
}

fn make_task(plan: &SourcePlan, now: Timestamp) -> Result<AccessTask, PlannerError> {
    Ok(AccessTask {
        task_id: TaskId::new(format!("t_{}", ulid::Ulid::new()))
            .map_err(|source| PlannerError::with_source("任务 ID", source))?,
        source_id: plan.source_id.clone(),
        accessor: plan.accessor.clone(),
        locator: plan.locator.clone(),
        spec: plan.spec.clone(),
        reason: TaskReason::Schedule,
        priority: priority(TaskReason::Schedule),
        queue: plan.queue,
        lease_until: now,
        attempt: 0,
        batch_key: plan.batch_key.clone(),
        generation: plan.generation,
    })
}

fn prepare_source(db: &mut Db, plan: &SourcePlan, today: &str, now: Timestamp) {
    let id = plan.source_id.as_str().to_string();
    let yield_7d = yield_sum(&db.yields, &id, today, 0, 7);
    let row = db
        .sources
        .entry(id)
        .or_insert_with(|| new_row(millis(plan.interval), plan.generation));
    let subject = format!("source/{}", plan.source_id);
    if let Some(health) = db.health.get_mut(&subject) {
        if health.status == "misconfigured" && row.generation != plan.generation {
            health.status = "ok".into();
            health.config = 0;
            row.next_at = stamp(now).ok();
            row.generation = plan.generation;
        }
    }
    row.generation = plan.generation;
    if row.last_recompute.as_deref() != Some(today) {
        row.interval_ms = millis(next_interval(&CadenceInput {
            yield_7d,
            floor: plan.floor,
            ceiling: plan.ceiling,
            signal: Duration::ZERO,
            paid: plan.paid,
            adaptive: plan.adaptive,
            fixed: plan.interval,
        }));
        row.last_recompute = Some(today.to_string());
    }
}

fn due_ids(db: &Db, plans: &[SourcePlan], now: Timestamp) -> BTreeSet<String> {
    let mut due = BTreeSet::new();
    for plan in plans {
        if !dispatchable(db, plan) {
            continue;
        }
        let due_now = db
            .sources
            .get(plan.source_id.as_str())
            .and_then(|row| row.next_at.as_deref())
            .map(|text| Timestamp::parse(text).map(|at| at <= now).unwrap_or(true))
            .unwrap_or(true);
        if due_now {
            due.insert(plan.source_id.as_str().to_string());
        }
    }
    let keys: Vec<String> = due
        .iter()
        .filter_map(|id| {
            plans
                .iter()
                .find(|plan| plan.source_id.as_str() == id)
                .and_then(|plan| plan.batch_key.clone())
        })
        .collect();
    for plan in plans {
        if plan
            .batch_key
            .as_ref()
            .is_some_and(|key| keys.iter().any(|item| item == key))
            && dispatchable(db, plan)
        {
            due.insert(plan.source_id.as_str().to_string());
        }
    }
    due
}

fn dispatchable(db: &Db, plan: &SourcePlan) -> bool {
    plan.enabled
        && !matches!(
            db.health
                .get(&format!("source/{}", plan.source_id))
                .map(|health| health.status.as_str()),
            Some("paused" | "misconfigured")
        )
}

fn status_of(db: &Db, source: &str, now: Timestamp) -> SourceStatus {
    let row = db.sources.get(source);
    let health = db.health.get(&format!("source/{source}"));
    let today = day_of(now);
    SourceStatus {
        next_at: row.and_then(|row| row.next_at.clone()),
        interval_ms: row.map(|row| row.interval_ms).unwrap_or(0),
        health_status: health
            .map(|item| item.status.clone())
            .unwrap_or_else(|| "ok".into()),
        alert: health.is_some_and(|item| item.alert),
        skip_reason: row.and_then(|row| row.skip_reason.clone()),
        budget_used: row.map(|row| row.paid_used).unwrap_or(0),
        last_error: health.and_then(|item| item.last_error.clone()),
        last_ok_at: health.and_then(|item| item.last_ok_at.clone()),
        last_yield_at: row.and_then(|row| row.last_yield_at.clone()),
        yields_7d: yield_sum(&db.yields, source, &today, 0, 7),
    }
}

pub(crate) fn yield_sum(
    yields: &BTreeMap<String, BTreeMap<String, u64>>,
    source: &str,
    today: &str,
    skip: i64,
    len: i64,
) -> u64 {
    let Some(days) = yields.get(source) else {
        return 0;
    };
    (skip..skip + len)
        .filter_map(|back| days_before(today, back))
        .filter_map(|day| days.get(&day).copied())
        .sum()
}

pub(crate) fn day_of(now: Timestamp) -> String {
    stamp(now).unwrap_or_default().chars().take(10).collect()
}

pub(crate) fn days_before(day: &str, back: i64) -> Option<String> {
    let format = time::format_description::parse_borrowed::<2>("[year]-[month]-[day]").ok()?;
    let date = time::Date::parse(day, &format).ok()?;
    let shifted = date.checked_sub(time::Duration::days(back))?;
    shifted.format(&format).ok()
}

pub(crate) fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

pub(crate) fn lease_span(timeout: Duration) -> Duration {
    timeout.saturating_mul(3) / 2
}

pub(crate) fn stamp(now: Timestamp) -> Result<String, PlannerError> {
    now.to_rfc3339()
        .map_err(|source| PlannerError::with_source("时间", source))
}

pub(crate) fn locator_key(locator: &Locator) -> Result<String, PlannerError> {
    serde_json::to_string(locator).map_err(|source| PlannerError::with_source("定位符", source))
}

pub(crate) async fn load_db<P: Persist>(persist: &P) -> Result<Db, PlannerError> {
    let Some(bytes) = persist.load("state").await? else {
        return Ok(Db::default());
    };
    serde_json::from_slice(&bytes)
        .map_err(|source| PlannerError::with_source("控制热数据读写失败", source))
}

pub(crate) async fn save_db<P: Persist>(persist: &P, db: &Db) -> Result<(), PlannerError> {
    let bytes = serde_json::to_vec(db)
        .map_err(|source| PlannerError::with_source("控制热数据读写失败", source))?;
    persist.store("state", &bytes).await
}
