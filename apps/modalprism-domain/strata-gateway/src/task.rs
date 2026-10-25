//! 任务队列。入队按 `source_id + locator_key` 去重，租约带过期时间。
//!
//! 状态有四种：`queued`、`leased`、`done`、`cancelled`。租约本身不增加尝试次数，
//! `nack` 才记账；过期租约可以被重新领取。`done` 与 `cancelled` 是终态，
//! 既不参与租约，也不再占住去重位。

use std::collections::BTreeMap;
use std::time::Duration;

use strata_model::{AccessTask, Locator, QueueName, TaskId, TaskReason, Timestamp};

use crate::db::{Db, Row, Tx};
use crate::error::{Error, Result};
use crate::sql::Sql;

pub const STATUS_QUEUED: &str = "queued";
pub const STATUS_LEASED: &str = "leased";
pub const STATUS_DONE: &str = "done";
pub const STATUS_CANCELLED: &str = "cancelled";

/// 终态：不再参与租约，也不再占住同一目标的去重位。
fn terminal(status: &str) -> bool {
    status == STATUS_DONE || status == STATUS_CANCELLED
}

/// SQL 里排除终态的唯一写法，避免状态字面量散落各处。
fn push_active(sql: &mut Sql) {
    sql.push(" AND status NOT IN (");
    sql.text(STATUS_DONE.to_string());
    sql.push(", ");
    sql.text(STATUS_CANCELLED.to_string());
    sql.push(")");
}

/// 列表与清理条件里的状态是否已知。
pub fn known_status(status: &str) -> bool {
    status == STATUS_QUEUED
        || status == STATUS_LEASED
        || status == STATUS_DONE
        || status == STATUS_CANCELLED
}

/// 列表条件里的原因是否已知。
pub fn known_reason(reason: &str) -> bool {
    [
        TaskReason::Schedule,
        TaskReason::Follow,
        TaskReason::RenderUpgrade,
        TaskReason::Detail,
        TaskReason::Manual,
        TaskReason::Replay,
    ]
    .iter()
    .any(|item| reason_text(*item) == reason)
}

/// 定位键。与 planner 端口一致，用定位符的 JSON 文本：
/// 同一 source 下同一目标只允许一个活跃任务。
pub fn locator_key(locator: &Locator) -> Result<String> {
    Ok(serde_json::to_string(locator)?)
}

pub fn queue_text(queue: QueueName) -> &'static str {
    queue.label()
}

pub fn reason_text(reason: TaskReason) -> &'static str {
    match reason {
        TaskReason::Schedule => "schedule",
        TaskReason::Follow => "follow",
        TaskReason::RenderUpgrade => "render_upgrade",
        TaskReason::Detail => "detail",
        TaskReason::Manual => "manual",
        TaskReason::Replay => "replay",
    }
}

fn stamp(now: Timestamp) -> Result<String> {
    Ok(now.to_rfc3339()?)
}

fn lease_until(now: Timestamp, lease_for: Duration) -> Result<String> {
    let until = now.checked_add(lease_for)?;
    Ok(until.to_rfc3339()?)
}

fn parse_task(row: &crate::db::Row) -> Result<AccessTask> {
    let mut task: AccessTask = serde_json::from_str(&row.text("payload")?)?;
    task.attempt = row.u64("attempt")? as u32;
    if let Some(until) = row.opt_text("lease_until")? {
        task.lease_until = Timestamp::parse(&until)?;
    }
    Ok(task)
}

fn returning_columns() -> &'static str {
    "RETURNING task_id, payload, attempt, lease_until, generation"
}

/// 租约是否到期：完成的任务不再参与，未到期的租约跳过。
fn lease_due(status: &str, lease_until: Option<&str>, now: Timestamp) -> Result<bool> {
    if terminal(status) {
        return Ok(false);
    }
    match lease_until {
        Some(text) => Ok(Timestamp::parse(text)? <= now),
        None => Ok(true),
    }
}

/// 入队。已有同定位活跃任务时跳过；返回实际写入的任务。
pub async fn enqueue(db: &Db, tasks: &[AccessTask]) -> Result<Vec<AccessTask>> {
    if tasks.is_empty() {
        return Ok(Vec::new());
    }
    let dialect = db.dialect();
    let mut tx = db.begin().await?;
    let mut inserted = Vec::new();
    for task in tasks {
        task.validate()?;
        let key = locator_key(&task.locator)?;
        if active_in(&mut tx, task.source_id.as_str(), &key).await? {
            continue;
        }
        let mut sql = Sql::new(dialect);
        sql.push("INSERT INTO g_tasks(task_id, queue, payload, status, lease_until, retry_at, last_error, priority, created_at, source_id, locator_key, attempt, generation, reason) VALUES (");
        sql.text(task.task_id.as_str());
        sql.push(", ");
        sql.text(queue_text(task.queue));
        sql.push(", ");
        sql.text(serde_json::to_string(task)?);
        sql.push(", ");
        sql.text(STATUS_QUEUED);
        sql.push(", NULL, NULL, NULL, ");
        sql.int(task.priority as i64);
        sql.push(", ");
        sql.text(stamp(Timestamp::now())?);
        sql.push(", ");
        sql.text(task.source_id.as_str());
        sql.push(", ");
        sql.text(key);
        sql.push(", 0, ");
        sql.u64(task.generation);
        sql.push(", ");
        sql.text(reason_text(task.reason));
        sql.push(") ");
        sql.push(dialect.on_conflict_do_nothing("task_id"));
        if tx.exec(&sql).await? > 0 {
            inserted.push(task.clone());
        }
    }
    tx.commit().await?;
    Ok(inserted)
}

/// 完成一个任务。已完成的再确认报状态错误。
pub async fn ack(db: &Db, task_id: &TaskId) -> Result<()> {
    let mut tx = db.begin().await?;
    ack_in(&mut tx, task_id).await?;
    tx.commit().await?;
    Ok(())
}

/// 事务内确认任务，供提交与确认同批完成。
pub async fn ack_in(tx: &mut Tx<'_>, task_id: &TaskId) -> Result<()> {
    let mut select = Sql::new(tx.dialect());
    select.push("SELECT status FROM g_tasks WHERE task_id = ");
    select.text(task_id.as_str());
    let row = tx
        .fetch_one(&select)
        .await?
        .ok_or_else(|| Error::UnknownTask(task_id.to_string()))?;
    let status = row.text("status")?;
    if status == STATUS_DONE {
        return Err(Error::TaskState(task_id.to_string()));
    }
    // 已取消的任务仍然提交数据，但不把状态改回完成。
    if status == STATUS_CANCELLED {
        return Ok(());
    }
    let mut sql = Sql::new(tx.dialect());
    sql.push("UPDATE g_tasks SET status = ");
    sql.text(STATUS_DONE);
    sql.push(", lease_until = NULL WHERE task_id = ");
    sql.text(task_id.as_str());
    tx.exec(&sql).await?;
    Ok(())
}

async fn active_in(tx: &mut Tx<'_>, source: &str, key: &str) -> Result<bool> {
    let mut sql = Sql::new(tx.dialect());
    sql.push("SELECT CAST(COUNT(*) AS BIGINT) AS n FROM g_tasks WHERE source_id = ");
    sql.text(source.to_string());
    sql.push(" AND locator_key = ");
    sql.text(key.to_string());
    push_active(&mut sql);
    Ok(tx.fetch_int(&sql).await?.unwrap_or(0) > 0)
}

pub async fn active(db: &Db, source: &str, key: &str) -> Result<bool> {
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT CAST(COUNT(*) AS BIGINT) AS n FROM g_tasks WHERE source_id = ");
    sql.text(source.to_string());
    sql.push(" AND locator_key = ");
    sql.text(key.to_string());
    push_active(&mut sql);
    Ok(db.fetch_int(&sql).await?.unwrap_or(0) > 0)
}

/// 批量租约：排队中或租约已过期的任务按优先级与入队时间领取。
pub async fn lease(
    db: &Db,
    queue: QueueName,
    count: usize,
    lease_for: Duration,
    now: Timestamp,
) -> Result<Vec<AccessTask>> {
    lease_with(db, queue, None, count, lease_for, now).await
}

/// 只领取派生任务（`reason = detail`），不会碰信源调度与跟进任务。
pub async fn lease_derivations(
    db: &Db,
    queue: QueueName,
    count: usize,
    lease_for: Duration,
    now: Timestamp,
) -> Result<Vec<AccessTask>> {
    lease_with(db, queue, Some(TaskReason::Detail), count, lease_for, now).await
}

/// 只领取跟进任务（`reason = follow`）。
pub async fn lease_follows(
    db: &Db,
    queue: QueueName,
    count: usize,
    lease_for: Duration,
    now: Timestamp,
) -> Result<Vec<AccessTask>> {
    lease_with(db, queue, Some(TaskReason::Follow), count, lease_for, now).await
}

async fn lease_with(
    db: &Db,
    queue: QueueName,
    reason: Option<TaskReason>,
    count: usize,
    lease_for: Duration,
    now: Timestamp,
) -> Result<Vec<AccessTask>> {
    if count == 0 {
        return Ok(Vec::new());
    }
    let dialect = db.dialect();
    let now_text = stamp(now)?;
    let mut sql = Sql::new(dialect);
    sql.push("UPDATE g_tasks SET status = ");
    sql.text(STATUS_LEASED);
    sql.push(", lease_until = ");
    sql.text(lease_until(now, lease_for)?);
    sql.push(" WHERE task_id IN (SELECT task_id FROM g_tasks WHERE queue = ");
    sql.text(queue_text(queue));
    if let Some(reason) = reason {
        sql.push(" AND reason = ");
        sql.text(reason_text(reason));
    }
    sql.push(" AND status IN ('queued', 'leased') AND (lease_until IS NULL OR lease_until <= ");
    sql.text(now_text.clone());
    sql.push(") AND (retry_at IS NULL OR retry_at <= ");
    sql.text(now_text);
    sql.push(") ORDER BY priority DESC, created_at ASC LIMIT ");
    sql.u64(count as u64);
    sql.push(") ");
    sql.push(returning_columns());
    let rows = db.fetch(&sql).await?;
    rows.iter().map(parse_task).collect()
}

/// 按定位键租一个特定任务。
pub async fn lease_ready(
    db: &Db,
    source: &str,
    key: &str,
    lease_for: Duration,
    now: Timestamp,
) -> Result<Option<AccessTask>> {
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT task_id FROM g_tasks WHERE source_id = ");
    sql.text(source.to_string());
    sql.push(" AND locator_key = ");
    sql.text(key.to_string());
    push_active(&mut sql);
    sql.push(" ORDER BY priority DESC, created_at ASC LIMIT 1");
    let Some(row) = db.fetch_one(&sql).await? else {
        return Ok(None);
    };
    let task_id = TaskId::new(row.text("task_id")?)?;
    match lease_id(db, &task_id, lease_for, now).await {
        Ok(task) => Ok(Some(task)),
        Err(Error::TaskState(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

/// 明确租一个任务。已完成、未到期的租约或尚未到重试时间的任务都报状态错误。
pub async fn lease_id(
    db: &Db,
    task_id: &TaskId,
    lease_for: Duration,
    now: Timestamp,
) -> Result<AccessTask> {
    let mut select = Sql::new(db.dialect());
    select.push("SELECT status, lease_until, retry_at FROM g_tasks WHERE task_id = ");
    select.text(task_id.as_str());
    let row = db
        .fetch_one(&select)
        .await?
        .ok_or_else(|| Error::UnknownTask(task_id.to_string()))?;
    let status = row.text("status")?;
    let lease = row.opt_text("lease_until")?;
    if !lease_due(&status, lease.as_deref(), now)? {
        return Err(Error::TaskState(task_id.to_string()));
    }
    if let Some(retry_at) = row.opt_text("retry_at")? {
        if Timestamp::parse(&retry_at)? > now {
            return Err(Error::TaskState(task_id.to_string()));
        }
    }
    let dialect = db.dialect();
    let mut sql = Sql::new(dialect);
    sql.push("UPDATE g_tasks SET status = ");
    sql.text(STATUS_LEASED);
    sql.push(", lease_until = ");
    sql.text(lease_until(now, lease_for)?);
    sql.push(" WHERE task_id = ");
    sql.text(task_id.as_str());
    sql.push(" ");
    sql.push(returning_columns());
    let row = db
        .fetch_one(&sql)
        .await?
        .ok_or_else(|| Error::UnknownTask(task_id.to_string()))?;
    parse_task(&row)
}

/// 续租。只对持有租约的任务有效。
pub async fn extend(db: &Db, task_id: &TaskId, lease_for: Duration, now: Timestamp) -> Result<()> {
    let mut select = Sql::new(db.dialect());
    select.push("SELECT status FROM g_tasks WHERE task_id = ");
    select.text(task_id.as_str());
    let Some(row) = db.fetch_one(&select).await? else {
        return Err(Error::UnknownTask(task_id.to_string()));
    };
    if row.text("status")? != STATUS_LEASED {
        return Err(Error::TaskState(task_id.to_string()));
    }
    let mut sql = Sql::new(db.dialect());
    sql.push("UPDATE g_tasks SET lease_until = ");
    sql.text(lease_until(now, lease_for)?);
    sql.push(" WHERE task_id = ");
    sql.text(task_id.as_str());
    db.exec(&sql).await?;
    Ok(())
}

/// 失败重试：回到排队状态，尝试次数加一并记录下一次可用时间。
pub async fn nack(db: &Db, task_id: &TaskId, retry_at: Timestamp, error: &str) -> Result<()> {
    let mut select = Sql::new(db.dialect());
    select.push("SELECT status FROM g_tasks WHERE task_id = ");
    select.text(task_id.as_str());
    let Some(row) = db.fetch_one(&select).await? else {
        return Err(Error::UnknownTask(task_id.to_string()));
    };
    if terminal(&row.text("status")?) {
        return Err(Error::TaskState(task_id.to_string()));
    }
    let dialect = db.dialect();
    let mut sql = Sql::new(dialect);
    sql.push("UPDATE g_tasks SET status = ");
    sql.text(STATUS_QUEUED);
    sql.push(", lease_until = NULL, attempt = attempt + 1, retry_at = ");
    sql.text(stamp(retry_at)?);
    sql.push(", last_error = ");
    sql.text(error.to_string());
    sql.push(" WHERE task_id = ");
    sql.text(task_id.as_str());
    db.exec(&sql).await?;
    Ok(())
}

/// 进程重启后回收所有租约。
pub async fn release_leases(db: &Db) -> Result<Vec<AccessTask>> {
    let dialect = db.dialect();
    let mut sql = Sql::new(dialect);
    sql.push("UPDATE g_tasks SET status = ");
    sql.text(STATUS_QUEUED);
    sql.push(", lease_until = NULL WHERE status = ");
    sql.text(STATUS_LEASED);
    sql.push(" ");
    sql.push(returning_columns());
    let rows = db.fetch(&sql).await?;
    let now = Timestamp::now();
    let mut tasks = Vec::with_capacity(rows.len());
    for row in &rows {
        let mut task = parse_task(row)?;
        task.lease_until = now;
        tasks.push(task);
    }
    Ok(tasks)
}

/// 当前未到期的租约数。
pub async fn leased_count(db: &Db, queue: QueueName, now: Timestamp) -> Result<usize> {
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT CAST(COUNT(*) AS BIGINT) AS n FROM g_tasks WHERE queue = ");
    sql.text(queue_text(queue));
    sql.push(" AND status = ");
    sql.text(STATUS_LEASED);
    sql.push(" AND lease_until > ");
    sql.text(stamp(now)?);
    Ok(db.fetch_int(&sql).await?.unwrap_or(0).max(0) as usize)
}

/// 热状态：小对象键值，用于去重与游标。
pub async fn hot_get(db: &Db, kind: &str, key: &str) -> Result<Option<String>> {
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT value FROM g_hot WHERE kind = ");
    sql.text(kind.to_string());
    sql.push(" AND key = ");
    sql.text(key.to_string());
    let row = db.fetch_one(&sql).await?;
    row.map(|row| row.text("value")).transpose()
}

pub async fn hot_put(db: &Db, kind: &str, key: &str, value: &str) -> Result<()> {
    let dialect = db.dialect();
    let mut sql = Sql::new(dialect);
    sql.push("INSERT INTO g_hot(kind, key, value) VALUES (");
    sql.text(kind.to_string());
    sql.push(", ");
    sql.text(key.to_string());
    sql.push(", ");
    sql.text(value.to_string());
    sql.push(") ON CONFLICT(kind, key) DO UPDATE SET value = excluded.value");
    db.exec(&sql).await?;
    Ok(())
}

/// 测试/运维：按状态计数。
pub async fn count_status(db: &Db, status: &str) -> Result<u64> {
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT CAST(COUNT(*) AS BIGINT) AS n FROM g_tasks WHERE status = ");
    sql.text(status.to_string());
    Ok(db.fetch_int(&sql).await?.unwrap_or(0).max(0) as u64)
}

/// 任务列表行：供控制台查看队列状态。
///
/// `spec` 里可能有访问凭据，永远不投影到这里。
#[derive(serde::Serialize)]
pub struct TaskRow {
    pub task_id: String,
    pub queue: String,
    pub status: String,
    pub source_id: String,
    pub accessor: String,
    pub reason: String,
    pub attempt: u64,
    pub priority: u64,
    pub generation: u64,
    pub created_at: String,
    pub retry_at: Option<String>,
    pub lease_until: Option<String>,
    pub last_error: Option<String>,
    pub locator: serde_json::Value,
}

/// 列表、统计与批量清理共用的条件。
#[derive(Clone, Debug, Default)]
pub struct TaskFilter {
    pub queue: Option<QueueName>,
    pub status: Option<String>,
    pub source: Option<String>,
    pub reason: Option<String>,
    /// 关键字：任务号、信源或错误信息里包含这段文字。
    pub query: Option<String>,
    /// 只处理创建时间早于此时刻的任务。
    pub before: Option<String>,
}

impl TaskFilter {
    fn push(&self, sql: &mut Sql) -> Result<()> {
        if let Some(queue) = self.queue {
            sql.push(" AND queue = ");
            sql.text(queue_text(queue));
        }
        if let Some(status) = self.status.as_deref().filter(|text| !text.is_empty()) {
            sql.push(" AND status = ");
            sql.text(status.to_string());
        }
        if let Some(source) = self.source.as_deref().filter(|text| !text.is_empty()) {
            sql.push(" AND source_id = ");
            sql.text(source.to_string());
        }
        if let Some(reason) = self.reason.as_deref().filter(|text| !text.is_empty()) {
            sql.push(" AND reason = ");
            sql.text(reason.to_string());
        }
        if let Some(query) = self
            .query
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            let pattern = format!("%{query}%");
            sql.push(" AND (task_id LIKE ");
            sql.text(pattern.clone());
            sql.push(" OR source_id LIKE ");
            sql.text(pattern.clone());
            sql.push(" OR last_error LIKE ");
            sql.text(pattern);
            sql.push(")");
        }
        if let Some(before) = self.before.as_deref().filter(|text| !text.is_empty()) {
            sql.push(" AND created_at < ");
            sql.text(Timestamp::parse(before)?.to_rfc3339()?);
        }
        Ok(())
    }
}

/// 一页任务：游标指向本页最后一行，`total` 是同一条件的总数。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskPage {
    pub rows: Vec<TaskRow>,
    pub next_cursor: Option<String>,
    pub total: u64,
}

/// 队列全景统计，供控制台直接展示而不必拉全量。
#[derive(Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStats {
    pub total: u64,
    pub by_status: BTreeMap<String, u64>,
    pub by_queue: BTreeMap<String, u64>,
    pub by_reason: BTreeMap<String, u64>,
    pub oldest_queued_at: Option<String>,
    pub next_retry_at: Option<String>,
    /// 租约已过期但尚未回收的任务数。
    pub leased_overdue: u64,
}

/// 按条件计数，供分页页脚与清理确认。
pub async fn count_tasks(db: &Db, filter: &TaskFilter) -> Result<u64> {
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT CAST(COUNT(*) AS BIGINT) AS n FROM g_tasks WHERE 1=1");
    filter.push(&mut sql)?;
    Ok(db.fetch_int(&sql).await?.unwrap_or(0).max(0) as u64)
}

/// 任务列表行与单条读取共用的列。`payload` 用于取 `accessor` 与 `locator`，
/// 其中的 `spec` 不进入列表投影。
const TASK_COLUMNS: &str = "task_id, queue, status, source_id, reason, attempt, priority, created_at, retry_at, lease_until, last_error, generation, payload";

fn row_of(row: &Row) -> Result<TaskRow> {
    let payload: serde_json::Value = serde_json::from_str(&row.text("payload")?)?;
    Ok(TaskRow {
        task_id: row.text("task_id")?.to_string(),
        queue: row.text("queue")?.to_string(),
        status: row.text("status")?.to_string(),
        source_id: row.text("source_id")?.to_string(),
        accessor: payload
            .get("accessor")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        reason: row.text("reason")?.to_string(),
        attempt: row.u64("attempt")?,
        priority: row.u64("priority")?,
        generation: row.u64("generation")?,
        created_at: row.text("created_at")?.to_string(),
        retry_at: row.opt_text("retry_at")?,
        lease_until: row.opt_text("lease_until")?,
        last_error: row.opt_text("last_error")?,
        locator: payload
            .get("locator")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    })
}

/// 单条任务行，供新建后回显。
pub async fn row(db: &Db, task_id: &TaskId) -> Result<Option<TaskRow>> {
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT ");
    sql.push(TASK_COLUMNS);
    sql.push(" FROM g_tasks WHERE task_id = ");
    sql.text(task_id.as_str());
    let Some(row) = db.fetch_one(&sql).await? else {
        return Ok(None);
    };
    Ok(Some(row_of(&row)?))
}

/// 任务列表：按条件过滤，新任务在前，`created_at + task_id` 做 keyset 游标。
pub async fn list_tasks(
    db: &Db,
    filter: &TaskFilter,
    cursor: Option<&str>,
    limit: usize,
) -> Result<TaskPage> {
    let limit = limit.max(1);
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT ");
    sql.push(TASK_COLUMNS);
    sql.push(" FROM g_tasks WHERE 1=1");
    filter.push(&mut sql)?;
    if let Some((at, id)) = cursor.and_then(|value| value.split_once('|')) {
        sql.push(" AND (created_at < ");
        sql.text(at.to_string());
        sql.push(" OR (created_at = ");
        sql.text(at.to_string());
        sql.push(" AND task_id < ");
        sql.text(id.to_string());
        sql.push("))");
    }
    sql.push(" ORDER BY created_at DESC, task_id DESC LIMIT ");
    sql.int(limit as i64 + 1);
    let rows = db.fetch(&sql).await?;
    let more = rows.len() > limit;
    let mut items = Vec::with_capacity(limit.min(rows.len()));
    for row in rows.iter().take(limit) {
        items.push(row_of(row)?);
    }
    let next_cursor = if more {
        items
            .last()
            .map(|row| format!("{}|{}", row.created_at, row.task_id))
    } else {
        None
    };
    let total = count_tasks(db, filter).await?;
    Ok(TaskPage {
        rows: items,
        next_cursor,
        total,
    })
}

/// 队列全景统计。
pub async fn stats(db: &Db) -> Result<TaskStats> {
    let by_status = group_count(db, "status").await?;
    let by_queue = group_count(db, "queue").await?;
    let by_reason = group_count(db, "reason").await?;
    let mut oldest = Sql::new(db.dialect());
    oldest.push("SELECT MIN(created_at) AS value FROM g_tasks WHERE status = ");
    oldest.text(STATUS_QUEUED.to_string());
    let oldest_queued_at = optional_text(db, &oldest).await?;
    let mut next = Sql::new(db.dialect());
    next.push("SELECT MIN(retry_at) AS value FROM g_tasks WHERE status = ");
    next.text(STATUS_QUEUED.to_string());
    next.push(" AND retry_at IS NOT NULL");
    let next_retry_at = optional_text(db, &next).await?;
    let mut overdue = Sql::new(db.dialect());
    overdue.push("SELECT CAST(COUNT(*) AS BIGINT) AS n FROM g_tasks WHERE status = ");
    overdue.text(STATUS_LEASED.to_string());
    overdue.push(" AND lease_until < ");
    overdue.text(stamp(Timestamp::now())?);
    let leased_overdue = db.fetch_int(&overdue).await?.unwrap_or(0).max(0) as u64;
    let total = by_status.values().sum();
    Ok(TaskStats {
        total,
        by_status,
        by_queue,
        by_reason,
        oldest_queued_at,
        next_retry_at,
        leased_overdue,
    })
}

/// 单个队列的计数，供控制台队列卡。
#[derive(Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueCounts {
    pub queue: String,
    pub queued: u64,
    pub leased: u64,
    pub done: u64,
    pub cancelled: u64,
}

/// 每队列 × 状态一次聚合。
pub async fn queue_counts(db: &Db) -> Result<Vec<QueueCounts>> {
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT queue AS key, status AS state, CAST(COUNT(*) AS BIGINT) AS n FROM g_tasks GROUP BY queue, status");
    let rows = db.fetch(&sql).await?;
    let mut out: BTreeMap<String, QueueCounts> = BTreeMap::new();
    for row in &rows {
        let queue = row.text("key")?;
        let count = row.u64("n")?;
        let slot = out.entry(queue.clone()).or_insert_with(|| QueueCounts {
            queue,
            ..QueueCounts::default()
        });
        match row.text("state")?.as_str() {
            STATUS_QUEUED => slot.queued = count,
            STATUS_LEASED => slot.leased = count,
            STATUS_DONE => slot.done = count,
            STATUS_CANCELLED => slot.cancelled = count,
            _ => {}
        }
    }
    Ok(out.into_values().collect())
}

async fn group_count(db: &Db, column: &str) -> Result<BTreeMap<String, u64>> {
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT ")
        .push(column)
        .push(" AS key, CAST(COUNT(*) AS BIGINT) AS n FROM g_tasks GROUP BY ")
        .push(column);
    let rows = db.fetch(&sql).await?;
    let mut out = BTreeMap::new();
    for row in &rows {
        let count = row.u64("n")?;
        if count > 0 {
            out.insert(row.text("key")?, count);
        }
    }
    Ok(out)
}

async fn optional_text(db: &Db, sql: &Sql) -> Result<Option<String>> {
    let Some(row) = db.fetch_one(sql).await? else {
        return Ok(None);
    };
    row.opt_text("value")
}

/// 取消：排队中或持租约的任务都可以取消，已终态的任务不动。
pub async fn cancel(db: &Db, task_id: &TaskId) -> Result<bool> {
    let mut sql = Sql::new(db.dialect());
    sql.push("UPDATE g_tasks SET status = ");
    sql.text(STATUS_CANCELLED);
    sql.push(", lease_until = NULL, retry_at = NULL, last_error = ");
    sql.text("已取消".to_string());
    sql.push(" WHERE task_id = ");
    sql.text(task_id.as_str());
    sql.push(" AND status IN (");
    sql.text(STATUS_QUEUED.to_string());
    sql.push(", ");
    sql.text(STATUS_LEASED.to_string());
    sql.push(")");
    Ok(db.exec(&sql).await? > 0)
}

/// 重试或恢复：排队、持租约、已取消的任务都回到可立即租约的状态。
pub async fn requeue(db: &Db, task_id: &TaskId) -> Result<bool> {
    let mut sql = Sql::new(db.dialect());
    sql.push("UPDATE g_tasks SET status = ");
    sql.text(STATUS_QUEUED);
    sql.push(", lease_until = NULL, retry_at = NULL WHERE task_id = ");
    sql.text(task_id.as_str());
    sql.push(" AND status IN (");
    sql.text(STATUS_QUEUED.to_string());
    sql.push(", ");
    sql.text(STATUS_LEASED.to_string());
    sql.push(", ");
    sql.text(STATUS_CANCELLED.to_string());
    sql.push(")");
    Ok(db.exec(&sql).await? > 0)
}

/// 删除单条终态任务。未终态的任务要求先取消。
pub async fn remove(db: &Db, task_id: &TaskId) -> Result<bool> {
    let mut select = Sql::new(db.dialect());
    select.push("SELECT status FROM g_tasks WHERE task_id = ");
    select.text(task_id.as_str());
    let Some(row) = db.fetch_one(&select).await? else {
        return Err(Error::UnknownTask(task_id.to_string()));
    };
    if !terminal(&row.text("status")?) {
        return Err(Error::TaskState(task_id.to_string()));
    }
    let mut sql = Sql::new(db.dialect());
    sql.push("DELETE FROM g_tasks WHERE task_id = ");
    sql.text(task_id.as_str());
    Ok(db.exec(&sql).await? > 0)
}

/// 按条件清理终态任务，返回删除条数。
pub async fn purge(db: &Db, filter: &TaskFilter) -> Result<u64> {
    if let Some(status) = filter.status.as_deref().filter(|text| !text.is_empty()) {
        if !terminal(status) {
            return Err(Error::InvalidFilter(format!("只能清理终态任务: {status}")));
        }
    }
    let mut sql = Sql::new(db.dialect());
    sql.push("DELETE FROM g_tasks WHERE status IN (");
    sql.text(STATUS_DONE.to_string());
    sql.push(", ");
    sql.text(STATUS_CANCELLED.to_string());
    sql.push(")");
    filter.push(&mut sql)?;
    Ok(db.exec(&sql).await? as u64)
}

/// 一条任务及其当前状态。仅供装配层执行任务用。
pub struct StoredTask {
    pub status: String,
    pub task: AccessTask,
}

/// 读取一条任务的完整内容（含 `spec`）。
///
/// 只给装配层执行任务用，不经能力层暴露——`spec` 里可能有凭据。
pub async fn read(db: &Db, task_id: &TaskId) -> Result<Option<StoredTask>> {
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT status, payload FROM g_tasks WHERE task_id = ");
    sql.text(task_id.as_str());
    let Some(row) = db.fetch_one(&sql).await? else {
        return Ok(None);
    };
    let task = serde_json::from_str(&row.text("payload")?)?;
    Ok(Some(StoredTask {
        status: row.text("status")?,
        task,
    }))
}
