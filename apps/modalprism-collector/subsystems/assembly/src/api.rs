//! 采集进程的窄接口。
//!
//! 能力层的 `logic/<method>.rs` 负责解析入参与映射错误，这里只接受领域类型，
//! 不出现 JSON 信封。这样子系统之间不靠字符串方法名和动态取值耦合。

use std::path::PathBuf;
use std::time::Duration;

use egress::Resolver;
use planner::SourceStatus;
use serde_json::Value;
use strata_client::TaskRow;
use strata_model::{
    AccessTask, ApplyOpts, ChangeSet, CommitResult, Producer, QueueName, TaskId, Timestamp,
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
        queue: Option<QueueName>,
        status: Option<&str>,
        limit: usize,
    ) -> Result<Vec<TaskRow>, Error> {
        Ok(self.strata.list_tasks(queue, status, limit).await?)
    }

    pub async fn queue_retry(&self, task_id: &TaskId) -> Result<bool, Error> {
        Ok(self.strata.retry_task(task_id).await?)
    }

    /// 落盘可见快照，返回新的快照号。
    pub async fn committer_flush(&self) -> Result<Option<u64>, Error> {
        Ok(self.strata.flush().await?)
    }

    pub async fn committer_recover(&self) -> Result<(), Error> {
        Ok(self.strata.recover().await?)
    }

    pub fn storage_paths(&self) -> StoragePaths {
        StoragePaths {
            control: self.data_dir.join("gateway.db"),
            blobs: self.data_dir.join("blobs"),
            staging: self.data_dir.join("staging"),
        }
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
    pub async fn runner_run(&self, task: AccessTask) -> Result<RunOutcome, Error> {
        let plans = plans::plans_of(&self.config.applied());
        let Some(mut plan) = plans
            .into_iter()
            .find(|plan| plan.source_id == task.source_id)
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
        let proxied = egress::route(self.proxy_setting(), &url).is_some();
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
