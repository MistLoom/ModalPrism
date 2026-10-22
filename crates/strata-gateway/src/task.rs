//! 任务队列。入队按 `source_id + locator_key` 去重，租约带过期时间。
//!
//! 与既有控制库语义一致：payload 存完整 `AccessTask`，队列/状态/租约落列，
//! 便于跨后端按列筛选。

use std::time::Duration;

use strata_model::{AccessTask, Locator, QueueName, TaskId, Timestamp};

use crate::db::Db;
use crate::error::{Error, Result};
use crate::sql::{Dialect, Sql};

pub const STATUS_QUEUED: &str = "queued";
pub const STATUS_LEASED: &str = "leased";

/// 定位键。同一 source 下同一目标只允许一个活跃任务。
pub fn locator_key(locator: &Locator) -> String {
    match locator {
        Locator::Url { url } => url.clone(),
        Locator::Query { query } => query.clone(),
        Locator::ApiEndpoint { endpoint } => endpoint.clone(),
        Locator::PushChannel { channel } => channel.clone(),
    }
}

pub fn queue_text(queue: QueueName) -> &'static str {
    match queue {
        QueueName::Free => "free",
        QueueName::Paid => "paid",
        QueueName::Render => "render",
        QueueName::Push => "push",
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
        let key = locator_key(&task.locator);
        if active_in(&mut tx, task.source_id.as_str(), &key).await? {
            continue;
        }
        let mut sql = Sql::new(dialect);
        sql.push("INSERT INTO g_tasks(task_id, queue, payload, status, lease_until, retry_at, last_error, priority, created_at, source_id, locator_key, attempt, generation) VALUES (");
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
        sql.push(") ");
        sql.push(dialect.on_conflict_do_nothing("task_id"));
        if tx.exec(&sql).await? > 0 {
            inserted.push(task.clone());
        }
    }
    tx.commit().await?;
    Ok(inserted)
}

/// 完成一个任务。
pub async fn ack(db: &Db, task_id: &TaskId) -> Result<()> {
    let dialect = db.dialect();
    let mut sql = Sql::new(dialect);
    sql.push("UPDATE g_tasks SET status = 'done', lease_until = NULL, retry_at = NULL WHERE task_id = ");
    sql.text(task_id.as_str());
    if db.exec(&sql).await? == 0 {
        return Err(Error::UnknownTask(task_id.to_string()));
    }
    Ok(())
}

async fn active_in(
    tx: &mut crate::db::Tx<'_>,
    source: &str,
    key: &str,
) -> Result<bool> {
    let mut sql = Sql::new(tx.dialect());
    sql.push("SELECT CAST(COUNT(*) AS BIGINT) AS n FROM g_tasks WHERE source_id = ");
    sql.text(source.to_string());
    sql.push(" AND locator_key = ");
    sql.text(key.to_string());
    sql.push(" AND status IN ('queued', 'leased')");
    Ok(tx.fetch_int(&sql).await?.unwrap_or(0) > 0)
}

pub async fn active(db: &Db, source: &str, key: &str) -> Result<bool> {
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT CAST(COUNT(*) AS BIGINT) AS n FROM g_tasks WHERE source_id = ");
    sql.text(source.to_string());
    sql.push(" AND locator_key = ");
    sql.text(key.to_string());
    sql.push(" AND status IN ('queued', 'leased')");
    Ok(db.fetch_int(&sql).await?.unwrap_or(0) > 0)
}

/// 批量租约：按优先级与入队时间取到期的排队任务。
pub async fn lease(
    db: &Db,
    queue: QueueName,
    count: usize,
    lease_for: Duration,
    now: Timestamp,
) -> Result<Vec<AccessTask>> {
    if count == 0 {
        return Ok(Vec::new());
    }
    let dialect = db.dialect();
    let until = lease_until(now, lease_for)?;
    let mut sql = Sql::new(dialect);
    sql.push("UPDATE g_tasks SET status = ");
    sql.text(STATUS_LEASED);
    sql.push(", lease_until = ");
    sql.text(until);
    sql.push(", attempt = attempt + 1 WHERE task_id IN (SELECT task_id FROM g_tasks WHERE queue = ");
    sql.text(queue_text(queue));
    sql.push(" AND status = ");
    sql.text(STATUS_QUEUED);
    sql.push(" AND (retry_at IS NULL OR retry_at <= ");
    sql.text(stamp(now)?);
    sql.push(") AND (lease_until IS NULL OR lease_until <= ");
    sql.text(stamp(now)?);
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
    let dialect = db.dialect();
    let mut sql = Sql::new(dialect);
    sql.push("UPDATE g_tasks SET status = ");
    sql.text(STATUS_LEASED);
    sql.push(", lease_until = ");
    sql.text(lease_until(now, lease_for)?);
    sql.push(", attempt = attempt + 1 WHERE task_id IN (SELECT task_id FROM g_tasks WHERE source_id = ");
    sql.text(source.to_string());
    sql.push(" AND locator_key = ");
    sql.text(key.to_string());
    sql.push(" AND status = ");
    sql.text(STATUS_QUEUED);
    sql.push(" ORDER BY priority DESC, created_at ASC LIMIT 1) ");
    sql.push(returning_columns());
    let rows = db.fetch(&sql).await?;
    rows.first().map(parse_task).transpose()
}

/// 明确租一个任务。
pub async fn lease_id(
    db: &Db,
    task_id: &TaskId,
    lease_for: Duration,
    now: Timestamp,
) -> Result<AccessTask> {
    let dialect = db.dialect();
    let mut sql = Sql::new(dialect);
    sql.push("UPDATE g_tasks SET status = ");
    sql.text(STATUS_LEASED);
    sql.push(", lease_until = ");
    sql.text(lease_until(now, lease_for)?);
    sql.push(", attempt = attempt + 1 WHERE task_id = ");
    sql.text(task_id.as_str());
    sql.push(" ");
    sql.push(returning_columns());
    let row = db
        .fetch_one(&sql)
        .await?
        .ok_or_else(|| Error::UnknownTask(task_id.to_string()))?;
    parse_task(&row)
}

/// 续租。
pub async fn extend(
    db: &Db,
    task_id: &TaskId,
    lease_for: Duration,
    now: Timestamp,
) -> Result<()> {
    let dialect = db.dialect();
    let mut sql = Sql::new(dialect);
    sql.push("UPDATE g_tasks SET lease_until = ");
    sql.text(lease_until(now, lease_for)?);
    sql.push(" WHERE task_id = ");
    sql.text(task_id.as_str());
    sql.push(" AND status = ");
    sql.text(STATUS_LEASED);
    if db.exec(&sql).await? == 0 {
        return Err(Error::TaskState(task_id.to_string()));
    }
    Ok(())
}

/// 失败重试：回到排队状态并记录重试时间。
pub async fn nack(
    db: &Db,
    task_id: &TaskId,
    retry_at: Timestamp,
    error: &str,
) -> Result<()> {
    let dialect = db.dialect();
    let mut sql = Sql::new(dialect);
    sql.push("UPDATE g_tasks SET status = ");
    sql.text(STATUS_QUEUED);
    sql.push(", lease_until = NULL, retry_at = ");
    sql.text(stamp(retry_at)?);
    sql.push(", last_error = ");
    sql.text(error.to_string());
    sql.push(" WHERE task_id = ");
    sql.text(task_id.as_str());
    if db.exec(&sql).await? == 0 {
        return Err(Error::UnknownTask(task_id.to_string()));
    }
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

/// 当前租约数。
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

#[allow(dead_code)]
fn unused(_: Dialect) {}