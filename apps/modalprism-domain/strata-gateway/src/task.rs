//! 任务队列。入队按 `source_id + locator_key` 去重，租约带过期时间。
//!
//! 状态只有三种：`queued`、`leased`、`done`。租约本身不增加尝试次数，
//! `nack` 才记账；过期租约可以被重新领取。

use std::time::Duration;

use strata_model::{AccessTask, Locator, QueueName, TaskId, TaskReason, Timestamp};

use crate::db::{Db, Tx};
use crate::error::{Error, Result};
use crate::sql::Sql;

pub const STATUS_QUEUED: &str = "queued";
pub const STATUS_LEASED: &str = "leased";
pub const STATUS_DONE: &str = "done";

/// 定位键。与 planner 端口一致，用定位符的 JSON 文本：
/// 同一 source 下同一目标只允许一个活跃任务。
pub fn locator_key(locator: &Locator) -> Result<String> {
    Ok(serde_json::to_string(locator)?)
}

pub fn queue_text(queue: QueueName) -> &'static str {
    match queue {
        QueueName::Free => "free",
        QueueName::Paid => "paid",
        QueueName::Render => "render",
        QueueName::Push => "push",
    }
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
    if status == STATUS_DONE {
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
    if row.text("status")? == STATUS_DONE {
        return Err(Error::TaskState(task_id.to_string()));
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
    sql.push(" AND status != 'done'");
    Ok(tx.fetch_int(&sql).await?.unwrap_or(0) > 0)
}

pub async fn active(db: &Db, source: &str, key: &str) -> Result<bool> {
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT CAST(COUNT(*) AS BIGINT) AS n FROM g_tasks WHERE source_id = ");
    sql.text(source.to_string());
    sql.push(" AND locator_key = ");
    sql.text(key.to_string());
    sql.push(" AND status != 'done'");
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
    sql.push(" AND status != 'done' ORDER BY priority DESC, created_at ASC LIMIT 1");
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
    if row.text("status")? == STATUS_DONE {
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
#[derive(serde::Serialize)]
pub struct TaskRow {
    pub task_id: String,
    pub queue: String,
    pub status: String,
    pub source_id: String,
    pub reason: String,
    pub attempt: u64,
    pub priority: u64,
    pub created_at: String,
    pub retry_at: Option<String>,
    pub lease_until: Option<String>,
    pub last_error: Option<String>,
    pub locator: serde_json::Value,
}

/// 任务列表：按队列与状态过滤，新任务在前。
pub async fn list_tasks(
    db: &Db,
    queue: Option<QueueName>,
    status: Option<&str>,
    limit: usize,
) -> Result<Vec<TaskRow>> {
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT task_id, queue, status, source_id, reason, attempt, priority, created_at, retry_at, lease_until, last_error, payload FROM g_tasks WHERE 1=1");
    if let Some(queue) = queue {
        sql.push(" AND queue = ");
        sql.text(queue_text(queue));
    }
    if let Some(status) = status {
        sql.push(" AND status = ");
        sql.text(status.to_string());
    }
    sql.push(" ORDER BY created_at DESC LIMIT ");
    sql.int(limit as i64);
    let rows = db.fetch(&sql).await?;
    let mut items = Vec::with_capacity(rows.len());
    for row in &rows {
        let payload: serde_json::Value = serde_json::from_str(&row.text("payload")?)?;
        items.push(TaskRow {
            task_id: row.text("task_id")?.to_string(),
            queue: row.text("queue")?.to_string(),
            status: row.text("status")?.to_string(),
            source_id: row.text("source_id")?.to_string(),
            reason: row.text("reason")?.to_string(),
            attempt: row.u64("attempt")?,
            priority: row.u64("priority")?,
            created_at: row.text("created_at")?.to_string(),
            retry_at: row.opt_text("retry_at")?,
            lease_until: row.opt_text("lease_until")?,
            last_error: row.opt_text("last_error")?,
            locator: payload
                .get("locator")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        });
    }
    Ok(items)
}

/// 手动重试：清空租约与重试时间，任务回到可立即租约的状态。
/// 仅排队中的任务允许重试；租约中的任务由持有人完成或退回。
pub async fn retry_task(db: &Db, task_id: &TaskId) -> Result<bool> {
    let mut sql = Sql::new(db.dialect());
    sql.push("UPDATE g_tasks SET status = ");
    sql.text(STATUS_QUEUED);
    sql.push(", lease_until = NULL, retry_at = NULL WHERE task_id = ");
    sql.text(task_id.as_str());
    sql.push(" AND status = ");
    sql.text(STATUS_QUEUED);
    Ok(db.exec(&sql).await? > 0)
}
