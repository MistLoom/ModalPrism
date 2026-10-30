//! 采集进程的窄接口。
//!
//! 能力层的 `logic/<method>.rs` 负责解析入参与映射错误，这里只接受领域类型，
//! 不出现 JSON 信封。这样子系统之间不靠字符串方法名和动态取值耦合。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use egress::Resolver;
use planner::SourceStatus;
use serde_json::Value;
use strata_client::{STATUS_CANCELLED, STATUS_DONE, TaskFilter, TaskPage, TaskRow, TaskStats};
use strata_model::{
    AccessTask, ApplyOpts, ChangeSet, CommitResult, Expr, Locator, Op, Producer, Query, QueryFrom,
    QueueName, Rid, SelectItem, Target, TaskId, TaskReason, Timestamp,
};

use crate::{
    Error, PaidReceipts, Services, TaskHandler, capacities, plans, policy_of, snapshot_interval,
};

/// 存储路径。`committer.status` 的形状由这里固定。
#[derive(serde::Serialize)]
pub struct StoragePaths {
    pub control: PathBuf,
    pub blobs: PathBuf,
    pub staging: PathBuf,
}

/// 存储现状：三个目录加各表行数。`committer.status` 的形状由这里固定。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitterStatus {
    #[serde(flatten)]
    pub paths: StoragePaths,
    pub counts: strata_client::StoreCounts,
}

/// 出站探针结果。
#[derive(serde::Serialize)]
pub struct EgressCheck {
    pub blocked: bool,
    pub proxied: bool,
    pub addresses: Vec<String>,
}

/// 上游代理。
#[derive(serde::Serialize)]
pub struct Upstream {
    pub url: String,
    pub source: String,
}

/// 出站报告：子系统视图加代理模式与上游。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EgressReport {
    #[serde(flatten)]
    pub views: egress::Report,
    pub mode: String,
    pub upstream: Option<Upstream>,
}

/// 出站面状态。
#[derive(serde::Serialize)]
pub struct EgressStatus {
    pub proxy: String,
    pub mode: &'static str,
    pub upstream: Option<Upstream>,
}

/// 一次任务执行的结果。
pub struct RunOutcome {
    pub nacked: Option<String>,
    pub follows: Vec<AccessTask>,
}

/// 一轮派发的结果。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoundOutcome {
    pub leased: usize,
    pub executed: Vec<String>,
    pub failed: Vec<(String, String)>,
    pub follows: usize,
}

/// 队列卡：每队列的排队、租约、并发上限与暂停标记。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueState {
    pub queue: String,
    pub queued: u64,
    pub leased: u64,
    pub done: u64,
    pub cancelled: u64,
    pub capacity: usize,
    pub paused: bool,
}

/// 内容筛选：信源、创建时间窗与是否包含已删除。
#[derive(Clone, Debug, Default)]
pub struct ContentFilter {
    pub source: Option<String>,
    pub before: Option<Timestamp>,
    pub after: Option<Timestamp>,
}

/// 匹配到的条数，`tombstoned` 是其中已删除的。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentCounts {
    pub matched: u64,
    pub tombstoned: u64,
}

/// 一次清理的结果。`truncated` 表示还有剩余，继续调用即可。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentPurgeReport {
    pub matched: u64,
    pub deleted: u64,
    pub truncated: bool,
    pub dry_run: bool,
}

/// 一次保留清理的结果：清掉的终态任务与逐表删除行数。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SweepReport {
    pub dry_run: bool,
    pub tasks: u64,
    /// 按调用方筛选命中的终态任务数（未给筛选时恒为 0）。
    pub filtered: u64,
    #[serde(flatten)]
    pub store: strata_client::PruneReport,
}

/// 内容翻页大小。
const CONTENT_PAGE: u32 = 500;
/// 单次清理上限，超出由前端继续调用。
const CONTENT_BATCH: usize = 2000;
/// 统计最多扫过的行数，防止一次点击拖垮控制库。
const CONTENT_COUNT_CAP: usize = 200_000;

impl ContentFilter {
    /// 筛选条件翻译成声明式表达式；空条件不产生 `where`。
    fn expr(&self) -> Result<Option<Expr>, Error> {
        let mut items = Vec::new();
        if let Some(source) = &self.source {
            items.push(serde_json::json!({"eq": ["sourceId", source]}));
        }
        if let Some(before) = self.before {
            items.push(serde_json::json!({"lt": ["createdAt", stamp(before)?]}));
        }
        if let Some(after) = self.after {
            items.push(serde_json::json!({"gte": ["createdAt", stamp(after)?]}));
        }
        let value = match items.len() {
            0 => return Ok(None),
            1 => items.remove(0),
            _ => serde_json::json!({ "all": items }),
        };
        Expr::try_from(value)
            .map(Some)
            .map_err(|error| Error::new(error.to_string()))
    }
}

fn stamp(at: Timestamp) -> Result<String, Error> {
    at.to_rfc3339()
        .map_err(|error| Error::new(error.to_string()))
}

/// 保留期换算成截止时刻。没配置或写成 `forever` 时这一项不清理。
fn deadline(value: Option<&Value>, now: Timestamp) -> Result<Option<Timestamp>, Error> {
    let Some(text) = value.and_then(Value::as_str) else {
        return Ok(None);
    };
    let Some(span) = config::duration(text) else {
        return Ok(None);
    };
    let span = i64::try_from(span.as_millis()).map_err(|_| Error::new("保留期过长"))?;
    let millis = now
        .unix_millis()
        .map_err(|error| Error::new(error.to_string()))?
        - span;
    Timestamp::from_unix_millis(millis)
        .map(Some)
        .map_err(|error| Error::new(error.to_string()))
}

/// `versions.facet_keep`：整数是保留版数，`all` 表示不裁，直接跳过。
fn facet_keep(value: Option<&Value>) -> BTreeMap<String, u64> {
    let mut keep = BTreeMap::new();
    let Some(Value::Object(map)) = value else {
        return keep;
    };
    for (kind, amount) in map {
        if let Some(count) = amount.as_u64() {
            keep.insert(kind.clone(), count);
        }
    }
    keep
}

impl Services {
    /// 数据面写入的唯一入口。
    pub async fn strata_apply(&self, changeset: ChangeSet) -> Result<CommitResult, Error> {
        Ok(self.strata.apply(changeset, ApplyOpts::default()).await?)
    }

    pub async fn queue_enqueue(&self, tasks: &[AccessTask]) -> Result<(), Error> {
        Ok(self.strata.enqueue(tasks).await?)
    }

    pub async fn queue_lease(
        &self,
        queue: QueueName,
        count: usize,
        lease_for: Duration,
    ) -> Result<Vec<AccessTask>, Error> {
        Ok(self.strata.lease(queue, count, lease_for).await?)
    }

    pub async fn queue_extend(&self, task_id: &TaskId, lease_for: Duration) -> Result<(), Error> {
        Ok(self.strata.extend(task_id, lease_for).await?)
    }

    pub async fn queue_nack(
        &self,
        task_id: &TaskId,
        retry_at: Timestamp,
        error: &str,
    ) -> Result<(), Error> {
        Ok(self.strata.nack(task_id, retry_at, error).await?)
    }

    pub async fn queue_list(
        &self,
        filter: &TaskFilter,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<TaskPage, Error> {
        Ok(self.strata.list_tasks(filter, cursor, limit).await?)
    }

    pub async fn queue_stats(&self) -> Result<TaskStats, Error> {
        Ok(self.strata.task_stats().await?)
    }

    pub async fn queue_requeue(&self, task_id: &TaskId) -> Result<bool, Error> {
        Ok(self.strata.requeue_task(task_id).await?)
    }

    pub async fn queue_cancel(&self, task_id: &TaskId) -> Result<bool, Error> {
        Ok(self.strata.cancel_task(task_id).await?)
    }

    pub async fn queue_remove(&self, task_id: &TaskId) -> Result<bool, Error> {
        Ok(self.strata.remove_task(task_id).await?)
    }

    pub async fn queue_purge(&self, filter: &TaskFilter) -> Result<u64, Error> {
        Ok(self.strata.purge_tasks(filter).await?)
    }

    pub async fn queue_count(&self, filter: &TaskFilter) -> Result<u64, Error> {
        Ok(self.strata.count_tasks(filter).await?)
    }

    /// 落盘可见快照，返回新的快照号。
    pub async fn committer_flush(&self) -> Result<Option<u64>, Error> {
        Ok(self.strata.flush().await?)
    }

    pub async fn committer_recover(&self) -> Result<(), Error> {
        Ok(self.strata.recover().await?)
    }

    /// 各表当前行数，给「保留与清理」卡片展示。
    pub async fn store_counts(&self) -> Result<strata_client::StoreCounts, Error> {
        Ok(self.strata.store_counts().await?)
    }

    /// 按 `policy` 的保留设置清理终态任务与过期存储行。
    ///
    /// 没配置的项一律不清理；`dry_run` 只统计不写。
    /// `filter` 是运维面的按筛选清理入口：只作用于终态任务，与保留策略叠加。
    pub async fn sweep(
        &self,
        dry_run: bool,
        filter: Option<TaskFilter>,
    ) -> Result<SweepReport, Error> {
        let filtered = match filter {
            Some(filter) => self.filtered_purge(&filter, dry_run).await?,
            None => 0,
        };
        let applied = self.config.applied();
        let system = &applied.system;
        let now = Timestamp::now();
        let tasks = match deadline(system.pointer("/policy/retention/tasks"), now)? {
            Some(before) => {
                let filter = TaskFilter {
                    before: Some(stamp(before)?),
                    ..Default::default()
                };
                if dry_run {
                    let mut total = 0;
                    for status in [strata_client::STATUS_DONE, strata_client::STATUS_CANCELLED] {
                        total += self
                            .queue_count(&TaskFilter {
                                status: Some(status.to_string()),
                                ..filter.clone()
                            })
                            .await?;
                    }
                    total
                } else {
                    self.queue_purge(&filter).await?
                }
            }
            None => 0,
        };
        let plan = strata_client::PrunePlan {
            snapshots_before: deadline(system.pointer("/policy/versions/snapshot_retain"), now)?,
            facet_keep: facet_keep(system.pointer("/policy/versions/facet_keep")),
            raw_before: deadline(system.pointer("/policy/raw/retain"), now)?,
        };
        let store = self.strata.prune(&plan, dry_run).await?;
        Ok(SweepReport {
            dry_run,
            tasks,
            filtered,
            store,
        })
    }

    /// 按调用方筛选的清理只能命中终态任务：未指定状态时逐个终态统计。
    async fn filtered_purge(&self, filter: &TaskFilter, dry_run: bool) -> Result<u64, Error> {
        if filter.status.is_some() {
            let matched = if dry_run {
                self.queue_count(filter).await?
            } else {
                self.queue_purge(filter).await?
            };
            return Ok(matched);
        }
        let mut total = 0u64;
        for status in [STATUS_DONE, STATUS_CANCELLED] {
            let scoped = TaskFilter {
                status: Some(status.to_string()),
                ..filter.clone()
            };
            if dry_run {
                total += self.queue_count(&scoped).await?;
            } else {
                total += self.queue_purge(&scoped).await?;
            }
        }
        Ok(total)
    }

    pub fn storage_paths(&self) -> StoragePaths {
        StoragePaths {
            control: self.data_dir.join("gateway.db"),
            blobs: self.data_dir.join("blobs"),
            staging: self.data_dir.join("staging"),
        }
    }

    /// 存储现状：路径加各表行数，一次调用给「保留与清理」卡片用。
    pub async fn committer_status(&self) -> Result<CommitterStatus, Error> {
        Ok(CommitterStatus {
            paths: self.storage_paths(),
            counts: self.store_counts().await?,
        })
    }

    pub async fn planner_tick(&self) -> Result<Vec<AccessTask>, Error> {
        let plans = plans::plans_of(&self.config.applied());
        Ok(self
            .planner
            .tick(&plans, &capacities(), Timestamp::now())
            .await?)
    }

    /// 把信源提前置为到期，然后立刻派发一轮。
    pub async fn planner_trigger(&self, source: &str) -> Result<Vec<AccessTask>, Error> {
        self.planner.make_due(source, Timestamp::now()).await?;
        self.planner_tick().await
    }

    pub async fn planner_next(&self, source: &str) -> Result<SourceStatus, Error> {
        Ok(self.planner.source_status(source).await?)
    }

    pub async fn planner_statuses(&self) -> Result<Vec<(String, SourceStatus)>, Error> {
        Ok(self.planner.source_statuses().await?)
    }

    /// 执行一个任务。信源已从配置里消失时改成 nack。
    ///
    /// 手动任务不依赖计划覆盖：未被任何一等计划覆盖的信源用 `plans::manual_plan`
    /// 合成一次性计划执行。定时任务只由落盘计划派发，取不到计划说明信源已从配置
    /// 消失，仍按 nack 退回。
    pub async fn runner_run(&self, task: AccessTask) -> Result<RunOutcome, Error> {
        let snapshot = self.config.applied();
        let Some(mut plan) = plans::plans_of(&snapshot)
            .into_iter()
            .find(|plan| plan.source_id == task.source_id)
            .or_else(|| match task.reason {
                TaskReason::Manual => plans::manual_plan(&snapshot, task.source_id.as_str()),
                _ => None,
            })
        else {
            self.strata
                .nack(&task.task_id, Timestamp::now(), "config")
                .await?;
            return Ok(RunOutcome {
                nacked: Some("config".into()),
                follows: Vec::new(),
            });
        };
        plan.generation = task.generation;
        let now = Timestamp::now();
        let deadline = now
            .checked_add(plan.timeout)
            .map_err(|source| Error::with_source("节奏", source))?;
        let handler = TaskHandler {
            state: self.fetch_state(plan.timeout),
            strata: self.strata(),
        };
        let receipts = PaidReceipts {
            paid: std::sync::Arc::clone(&self.paid),
        };
        let follows = self
            .planner
            .run(&plan, task, &handler, &receipts, now, deadline)
            .await?;
        Ok(RunOutcome {
            nacked: None,
            follows,
        })
    }

    /// 按任务号执行：排队中的任务先挂租约，已挂租约的直接执行。
    ///
    /// 这是「立即执行」的唯一入口——控制台只传任务号，不碰 `spec`。
    pub async fn run_task_by_id(&self, task_id: &TaskId) -> Result<RunOutcome, Error> {
        let stored = self
            .strata
            .read_task(task_id)
            .await?
            .ok_or_else(|| Error::new("任务不存在"))?;
        if stored.status == strata_client::STATUS_CANCELLED
            || stored.status == strata_client::STATUS_DONE
        {
            return Err(Error::new("任务已终态"));
        }
        let task = if stored.status == strata_client::STATUS_QUEUED {
            let lease_for = plans::plans_of(&self.config.applied())
                .into_iter()
                .find(|plan| plan.source_id == stored.task.source_id)
                .map(|plan| plan.timeout.saturating_mul(3) / 2)
                .unwrap_or_else(|| Duration::from_secs(60));
            self.strata
                .lease_id(task_id, lease_for, Timestamp::now())
                .await?
        } else {
            stored.task
        };
        self.runner_run(task).await
    }

    /// 控制台「立即执行」入口：按任务号跑一条既有任务并回显。
    /// 直接按 id 租约，绕开调度器的到期/容量/暂停闸门，用于补跑卡在排队的任务。
    pub async fn task_run(&self, task_id: &TaskId) -> Result<TaskRow, Error> {
        self.run_task_by_id(task_id).await?;
        self.task_row(task_id).await
    }

    /// 跑一轮：`Some(source)` 先把它置到期，`None` 只执行本轮已到期的。
    ///
    /// 定时循环与「立即采集」共用这一个入口，不另存一份执行编排。
    pub async fn dispatch_round(&self, source: Option<&str>) -> Result<RoundOutcome, Error> {
        if let Some(source) = source {
            self.planner.make_due(source, Timestamp::now()).await?;
        }
        let leased = self.planner_tick().await?;
        let picked: Vec<AccessTask> = match source {
            Some(source) => leased
                .into_iter()
                .filter(|task| task.source_id.as_str() == source)
                .collect(),
            None => leased,
        };
        let mut outcome = RoundOutcome {
            leased: picked.len(),
            executed: Vec::new(),
            failed: Vec::new(),
            follows: 0,
        };
        for task in picked {
            let id = task.task_id.to_string();
            match self.runner_run(task).await {
                Ok(run) => {
                    outcome.executed.push(id);
                    outcome.follows += run.follows.len();
                    for follow in run.follows {
                        let id = follow.task_id.to_string();
                        match self.runner_run(follow).await {
                            Ok(_) => outcome.executed.push(id),
                            Err(error) => outcome.failed.push((id, error.message)),
                        }
                    }
                }
                Err(error) => outcome.failed.push((id, error.message)),
            }
        }
        if outcome.follows > 0 {
            let _ = self.committer_flush().await;
        }
        Ok(outcome)
    }

    /// 为信源新建一条人工任务。控制台永远不自己拼任务体。
    ///
    /// 信源未被任何一等计划覆盖时，用 `plans::manual_plan` 合成一次性计划派发，
    /// 手动采集因此不依赖计划覆盖；自动采集仍只认落盘计划。
    ///
    /// `locator` 提供时把取数目标换成这个地址（点位采集），其余字段沿用信源声明；
    /// 只对按地址取数的信源开放，其它形态（查询、接口端点、推送）拒绝。
    pub async fn task_create(
        &self,
        source: &str,
        queue: Option<QueueName>,
        run_now: bool,
        locator: Option<String>,
    ) -> Result<TaskRow, Error> {
        let applied = self.config.applied();
        let mut plan = plans::plans_of(&applied)
            .into_iter()
            .find(|plan| plan.source_id.as_str() == source)
            // 未被任何计划覆盖的信源仍要能手动采集：就地合成一条一次性计划，
            // 只用于本次即时派发，不改变该信源的自动采集资格。
            .or_else(|| plans::manual_plan(&applied, source))
            .ok_or_else(|| Error::new("信源不可采集"))?;
        if let Some(queue) = queue {
            plan.queue = queue;
        }
        if let Some(url) = locator.filter(|text| !text.trim().is_empty()) {
            if !matches!(plan.locator, Locator::Url { .. }) {
                return Err(Error::new("该信源不是按地址取数，不能指定目标地址"));
            }
            plan.locator = Locator::Url { url };
        }
        let task = planner::make_task(&plan, TaskReason::Manual, Timestamp::now())?;
        let new_id = task.task_id.clone();
        let source_str = plan.source_id.as_str().to_string();
        let locator_json = task.locator_key();
        self.strata.enqueue(std::slice::from_ref(&task)).await?;
        // 入队对同 source+locator 的活跃任务去重：这条新任务可能被跳过而不入库。
        // 解析出真正活跃的那条，避免对不存在的新 id 直接 run/回读而误报「任务不存在」。
        let effective_id = match self.strata.task_row(&new_id).await? {
            Some(_) => new_id,
            None => self
                .strata
                .active_task_id(&source_str, &locator_json)
                .await?
                .ok_or_else(|| Error::new("信源已有活跃任务"))?,
        };
        if run_now {
            self.run_task_by_id(&effective_id).await?;
        }
        self.task_row(&effective_id).await
    }

    /// 读出单条任务用于回显。
    async fn task_row(&self, task_id: &TaskId) -> Result<TaskRow, Error> {
        self.strata
            .task_row(task_id)
            .await?
            .ok_or_else(|| Error::new("任务不存在"))
    }

    /// 每队列的排队、租约、上限与暂停状态。
    pub async fn queue_states(&self) -> Result<Vec<QueueState>, Error> {
        let counts = self.strata.queue_counts().await?;
        let paused = self.planner.paused().await?;
        let room = capacities();
        let mut states: Vec<QueueState> = [
            QueueName::Free,
            QueueName::Paid,
            QueueName::Render,
            QueueName::Push,
        ]
        .into_iter()
        .map(|queue| QueueState {
            queue: queue.label().to_string(),
            queued: 0,
            leased: 0,
            done: 0,
            cancelled: 0,
            capacity: room.capacity(queue),
            paused: paused.contains(queue.label()),
        })
        .collect();
        for count in counts {
            if let Some(state) = states.iter_mut().find(|state| state.queue == count.queue) {
                state.queued = count.queued;
                state.leased = count.leased;
                state.done = count.done;
                state.cancelled = count.cancelled;
            }
        }
        Ok(states)
    }

    /// 暂停或恢复一个队列。
    pub async fn queue_pause(&self, queue: QueueName, paused: bool) -> Result<(), Error> {
        self.planner.set_paused(queue, paused).await?;
        Ok(())
    }

    /// 内容统计：匹配条数与其中已删除的条数。
    pub async fn content_count(&self, filter: &ContentFilter) -> Result<ContentCounts, Error> {
        let (rows, _) = self.content_rows(filter, CONTENT_COUNT_CAP).await?;
        Ok(ContentCounts {
            matched: rows.len() as u64,
            tombstoned: rows.iter().filter(|(_, tombstoned)| *tombstoned).count() as u64,
        })
    }

    /// 物理删除匹配到的内容。`dry_run` 只统计不删，超过单批上限时 `truncated` 为真。
    pub async fn content_delete(
        &self,
        filter: &ContentFilter,
        dry_run: bool,
    ) -> Result<ContentPurgeReport, Error> {
        let (rows, truncated) = self.content_rows(filter, CONTENT_BATCH).await?;
        let rids: Vec<Rid> = rows.iter().map(|(rid, _)| rid.clone()).collect();
        let deleted = if dry_run {
            rids.len() as u64
        } else {
            self.content_delete_rids(&rids).await?
        };
        Ok(ContentPurgeReport {
            matched: rows.len() as u64,
            deleted,
            truncated,
            dry_run,
        })
    }

    /// 按 rid 物理删除内容，返回真正移除的资源数。
    pub async fn content_delete_rids(&self, rids: &[Rid]) -> Result<u64, Error> {
        let report = self.strata.purge_resources(rids).await?;
        Ok(report.tables.get("g_resources").copied().unwrap_or(0))
    }

    /// 用户分类打标：对多条 rid 同时设置或撤销标签，返回打标条数。
    pub async fn content_label(
        &self,
        rids: &[Rid],
        set: &BTreeMap<String, String>,
        unset: &[String],
    ) -> Result<u64, Error> {
        if set.is_empty() && unset.is_empty() {
            return Err(Error::new("没有要写入或撤销的标签"));
        }
        let ops = rids
            .iter()
            .map(|rid| Op::Label {
                target: Target::Rid(rid.clone()),
                set: set.clone(),
                unset: unset.to_vec(),
            })
            .collect();
        self.content_commit(ops).await?;
        Ok(rids.len() as u64)
    }

    /// 内容治理写入的唯一出口：一次提交加一次刷盘。
    async fn content_commit(&self, ops: Vec<Op>) -> Result<(), Error> {
        if ops.is_empty() {
            return Ok(());
        }
        self.strata
            .apply(
                ChangeSet {
                    producer: Producer {
                        module: "content_admin".into(),
                        version: "1".into(),
                        config_generation: self.config_generation(),
                    },
                    idempotency_key: None,
                    ack: None,
                    ops,
                },
                ApplyOpts::default(),
            )
            .await?;
        self.committer_flush().await?;
        Ok(())
    }

    /// 按筛选翻页取出 `(rid, 已删除)`，最多 `cap` 条。
    async fn content_rows(
        &self,
        filter: &ContentFilter,
        cap: usize,
    ) -> Result<(Vec<(Rid, bool)>, bool), Error> {
        let mut rows = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let page = self
                .strata
                .query(Query {
                    from: QueryFrom::Resources,
                    r#where: filter.expr()?,
                    has_facets: Vec::new(),
                    select: vec![
                        SelectItem::Field("rid".into()),
                        SelectItem::Field("tombstone".into()),
                    ],
                    expand: Vec::new(),
                    search: None,
                    as_of: None,
                    order_by: Vec::new(),
                    limit: Some(CONTENT_PAGE),
                    cursor: cursor.clone(),
                    include_tombstoned: true,
                })
                .await?;
            for row in &page.rows {
                let rid = row
                    .get("rid")
                    .and_then(Value::as_str)
                    .ok_or_else(|| Error::new("内容行缺少 rid"))?;
                rows.push((
                    Rid::parse(rid).map_err(|error| Error::new(error.to_string()))?,
                    row.get("tombstone")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                ));
                if rows.len() >= cap {
                    return Ok((rows, true));
                }
            }
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => return Ok((rows, false)),
            }
        }
    }

    /// 汇总健康快照并写入存储。
    pub async fn health_evaluate(&self) -> Result<(), Error> {
        let now = Timestamp::now();
        let applied = self.config.applied();
        let ops = self
            .planner
            .snapshot(now, snapshot_interval(&applied))
            .await?;
        if ops.is_empty() {
            return Ok(());
        }
        self.strata
            .apply(
                ChangeSet {
                    producer: Producer {
                        module: "health".into(),
                        version: "1".into(),
                        config_generation: applied.generation,
                    },
                    idempotency_key: None,
                    ack: None,
                    ops,
                },
                ApplyOpts::default(),
            )
            .await?;
        self.planner.commit_snapshot(now).await?;
        Ok(())
    }

    pub fn config_generation(&self) -> u64 {
        self.config.snapshot().generation
    }

    /// 重新读取配置目录，返回重新读取后的代数。
    pub fn config_reload(&self) -> Result<u64, Error> {
        self.config.reload()?;
        Ok(self.config.snapshot().generation)
    }

    pub async fn egress_check(&self, raw: &str) -> Result<EgressCheck, Error> {
        let url = url::Url::parse(raw).map_err(|_| Error::new("gateway"))?;
        let host = url.host_str().ok_or_else(|| Error::new("gateway"))?;
        let proxied = egress::route(&self.proxy_setting(), &url).is_some();
        let addresses = crate::SystemResolver
            .lookup(host)
            .await
            .map_err(|error| Error::with_source(error.message.clone(), error))?;
        let policy = policy_of(&self.config.applied().system);
        let blocked = if proxied || policy.private_networks == egress::PrivateNetworks::Allow {
            false
        } else {
            addresses.is_empty() || addresses.iter().all(|ip| egress::is_blocked(*ip))
        };
        Ok(EgressCheck {
            blocked,
            proxied,
            addresses: addresses.iter().map(ToString::to_string).collect(),
        })
    }

    pub fn egress_status(&self) -> EgressStatus {
        EgressStatus {
            proxy: self.proxy_url(),
            mode: self.proxy_setting().label(),
            upstream: self.upstream_proxy().map(|route| Upstream {
                url: route.display(),
                source: route.source.clone(),
            }),
        }
    }

    /// 出站子系统自带的报告，附加代理模式与上游。
    pub async fn egress_report(&self) -> Result<EgressReport, Error> {
        let views = self
            .egress
            .report()
            .await
            .map_err(|error| Error::with_source(error.message.clone(), error))?;
        let status = self.egress_status();
        Ok(EgressReport {
            views,
            mode: status.mode.to_string(),
            upstream: status.upstream,
        })
    }

    pub async fn egress_reset(&self, host: &str) -> Result<bool, Error> {
        self.egress_gateway()
            .reset_circuit(host)
            .await
            .map_err(|error| Error::with_source(error.message.clone(), error))
    }

    pub async fn paid_complete(&self, receipt: &str) -> Result<(), Error> {
        self.paid
            .complete(receipt)
            .await
            .map_err(|error| Error::with_source(error.message.clone(), error))
    }

    pub async fn paid_status(&self, receipt: &str) -> Result<Option<String>, Error> {
        self.paid
            .status(receipt)
            .await
            .map_err(|error| Error::with_source(error.message.clone(), error))
    }

    /// 按 `implementation` 打开 accessor 并校验 spec。
    pub fn accessor_validate(&self, implementation: &str, spec: &Value) -> Result<(), Error> {
        let browser = self.browser_program.lock().expect("浏览器").clone();
        let accessor = crate::execute::open_accessor(implementation, &browser, &self.proxy_url())
            .map_err(|error| Error::new(error.message))?;
        accessor
            .validate(spec)
            .map_err(|error| Error::new(error.message))
    }
}
