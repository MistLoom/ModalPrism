//! 采集进程的存储、配置、调度和监听装配。

mod api;
mod derive;
mod error;
mod execute;
mod listen;
mod plans;
mod queue;
mod recover;
mod stage;
mod transport;
/// 工具面的单次采集：取数 → 识别 → 适配 → 落库/返回。
pub mod tools;

use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use config::{ConfigStore, LoadOptions};
use egress::TcpDial;
use planner::{Handler, Planner, Receipts};
use serde_json::Value;
use strata_client::{Options, Strata};
use strata_model::{
    ApplyOpts, ChangeSet, FacetKind, FacetRef, Locator, Producer, TaskReason, Timestamp, VersionSel,
};

pub use api::{
    CommitterStatus, ContentCounts, ContentFilter, ContentPurgeReport, EgressCheck, EgressStatus,
    QueueState, RoundOutcome, RunOutcome, StoragePaths, SweepReport, Upstream,
};
pub use error::{Error, invalid_input};
pub use listen::{serve_proxy, serve_push};
pub use plans::{
    GateReport, capacities, dry_run_migration_gate, flush_interval, snapshot_interval,
};
pub use transport::{EgressGw, policy_of};

use execute::{FetchState, FollowWork, NoSession};
use queue::{Hot, StrataQueue};
use stage::DiskStaging;
use transport::{HttpTransport, PaidBridge, PaidGw, SystemResolver};

/// 领域类型出口。能力层只依赖 `assembly`，不直接依赖数据层 crate。
pub mod domain {
    pub use planner::SourceStatus;
    pub use strata_client::{TaskFilter, TaskPage, TaskRow, TaskStats, known_reason, known_status};
    pub use strata_model::{
        AccessTask, ChangeSet, CommitResult, QueueName, Rid, TaskId, Timestamp,
    };
}

/// 三个环回监听地址。
pub struct Binds {
    pub http: SocketAddr,
    pub proxy: SocketAddr,
    pub push: SocketAddr,
}

/// 拒绝非环回地址。不创建数据目录。
pub fn require_loopback(addr: SocketAddr) -> Result<(), Error> {
    if addr.ip().is_loopback() {
        Ok(())
    } else {
        Err(Error::new("监听地址不是环回地址"))
    }
}

pub struct Services {
    data_dir: PathBuf,
    config_dir: PathBuf,
    load_options: LoadOptions,
    config: Arc<ConfigStore>,
    strata: Arc<Strata>,
    planner: Planner<StrataQueue, Hot>,
    egress: Arc<EgressGw>,
    paid: Arc<PaidGw>,
    proxy: Mutex<String>,
    push: accessors_builtin::PushAccessor,
    browser_program: Mutex<PathBuf>,
    /// 结构化识别引擎（RawPrism）。组合根在 Host 就绪后注入；未注入表示能力缺失。
    prism: Mutex<Option<Arc<dyn adapters::Prism>>>,
}

impl Services {
    pub async fn open(data: &Path, config_dir: &Path, binds: Binds) -> Result<Arc<Self>, Error> {
        let load_options = LoadOptions::from_process();
        // 配置不可加载时先尝试用出厂快照自愈；仍失败才拒绝启动。
        let snapshot = recover::load_or_recover(data, config_dir, &load_options)?;
        strata_client::check_storage(&snapshot.system, data).map_err(Error::new)?;
        require_loopback(binds.http)?;
        require_loopback(binds.proxy)?;
        require_loopback(binds.push)?;
        let config = Arc::new(ConfigStore::open(config_dir, load_options.clone())?);
        let opening = policy_of(&snapshot.system);
        let strata = Arc::new(Strata::open(data, Options::from_system(&snapshot.system)?).await?);
        let planner = Planner::new(
            StrataQueue(Arc::clone(&strata)),
            Hot::new(Arc::clone(&strata), "planner"),
        );
        let egress = Arc::new(egress::Gateway::new(
            Hot::new(Arc::clone(&strata), "egress"),
            HttpTransport::new(Arc::clone(&config)),
            SystemResolver,
            TcpDial,
            opening.clone(),
        ));
        let proxy_setting = transport::proxy_setting(&snapshot.system);
        let upstream = egress::probe(&proxy_setting);
        tracing::info!(
            mode = proxy_setting.label(),
            upstream = upstream.as_ref().map(egress::ProxyRoute::display),
            "出站代理"
        );
        let paid = Arc::new(paid::Gateway::new(
            Hot::new(Arc::clone(&strata), "paid"),
            PaidBridge {
                egress: Arc::clone(&egress),
                fallback: opening,
            },
            transport::call_policy_from(&snapshot.system, &Value::Null),
        ));
        let browser = std::env::var("MODALPRISM_MOLI").unwrap_or_else(|_| "moli".into());
        Ok(Arc::new(Self {
            data_dir: normalize(data),
            config_dir: config_dir.to_path_buf(),
            load_options,
            config,
            strata,
            planner,
            egress,
            paid,
            proxy: Mutex::new(format!("http://{}", binds.proxy)),
            push: accessors_builtin::PushAccessor::new(),
            browser_program: Mutex::new(PathBuf::from(browser)),
            prism: Mutex::new(None),
        }))
    }

    pub fn config(&self) -> Arc<ConfigStore> {
        Arc::clone(&self.config)
    }

    pub fn strata(&self) -> Arc<Strata> {
        Arc::clone(&self.strata)
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// 配置目录：声明文件的唯一位置，由组合根交给领域 API。
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn set_proxy(&self, url: impl Into<String>) {
        *self.proxy.lock().expect("代理") = url.into();
    }

    /// 注入结构化识别引擎。只写一次：Host 就绪后由组合根接线。
    pub fn set_prism(&self, prism: Arc<dyn adapters::Prism>) {
        if let Ok(mut slot) = self.prism.lock() {
            *slot = Some(prism);
        }
    }

    /// 当前可用的识别引擎。未装载时为 None，调用方按基础检测降级。
    pub fn prism(&self) -> Option<Arc<dyn adapters::Prism>> {
        self.prism.lock().ok().and_then(|slot| slot.clone())
    }

    pub fn proxy_url(&self) -> String {
        self.proxy.lock().expect("代理").clone()
    }

    /// 全局代理设置。每次读取当前配置代际，改配置即生效。
    pub fn proxy_setting(&self) -> egress::ProxySetting {
        transport::proxy_setting(&self.config.applied().system)
    }

    /// 探测当前代际下的上游代理，供启动日志与状态接口展示。
    pub fn upstream_proxy(&self) -> Option<egress::ProxyRoute> {
        egress::probe(&self.proxy_setting())
    }

    pub fn set_browser_program(&self, program: impl Into<PathBuf>) {
        *self.browser_program.lock().expect("浏览器") = program.into();
    }

    pub async fn shutdown(&self) -> Result<(), Error> {
        self.planner.shutdown(Timestamp::now()).await?;
        Ok(())
    }

    pub(crate) fn fetch_state(&self, timeout: Duration) -> FetchState {
        FetchState {
            snapshot: self.config.applied(),
            egress: Arc::clone(&self.egress),
            paid: Arc::clone(&self.paid),
            staging_dir: self.data_dir.join("staging"),
            proxy: self.proxy_url(),
            browser: self.browser_program.lock().expect("浏览器").clone(),
            timeout,
            prism: self.prism.lock().ok().and_then(|slot| slot.clone()),
        }
    }

    pub(crate) fn access_context(&self) -> accessors::AccessContext {
        accessors::AccessContext {
            egress: self.egress.clone(),
            paid: self.paid.clone(),
            sessions: Arc::new(NoSession),
            staging: Arc::new(DiskStaging {
                dir: self.data_dir.join("staging"),
            }),
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    pub(crate) fn push(&self) -> &accessors_builtin::PushAccessor {
        &self.push
    }

    pub(crate) async fn commit_inbound(
        &self,
        source_id: &str,
        envelopes: &[strata_model::RawEnvelope],
    ) -> Result<(), Error> {
        let snapshot = self.config.applied();
        let module = snapshot
            .modules
            .get(source_id)
            .cloned()
            .ok_or_else(|| Error::new("config"))?;
        let state = self.fetch_state(Duration::from_secs(30));
        let ops = execute::inbound_ops(&state, source_id, &module, envelopes)
            .await
            .map_err(|error| Error::new(error.message))?;
        if ops.is_empty() {
            return Ok(());
        }
        self.strata
            .apply(
                ChangeSet {
                    producer: Producer {
                        module: "push".into(),
                        version: "1".into(),
                        config_generation: snapshot.generation,
                    },
                    idempotency_key: None,
                    ack: None,
                    ops,
                },
                ApplyOpts::default(),
            )
            .await?;
        self.strata.flush().await?;
        Ok(())
    }

    pub fn egress_gateway(&self) -> Arc<EgressGw> {
        Arc::clone(&self.egress)
    }
}

impl types::ReadCtx for Services {
    fn config(&self) -> &ConfigStore {
        self.config.as_ref()
    }

    fn strata(&self) -> &Strata {
        self.strata.as_ref()
    }

    fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    fn load_options(&self) -> &LoadOptions {
        &self.load_options
    }
}

struct TaskHandler {
    state: FetchState,
    strata: Arc<Strata>,
}

#[async_trait]
impl Handler for TaskHandler {
    async fn execute(
        &self,
        task: &strata_model::AccessTask,
        cursor: Option<Value>,
    ) -> Result<planner::TaskSuccess, planner::Classified> {
        let resource = if task.reason == TaskReason::Follow {
            let key = match &task.locator {
                Locator::Url { url } => format!("url:{url}"),
                _ => String::new(),
            };
            if key.is_empty() {
                None
            } else {
                self.strata.resolve(&key).await.ok().flatten()
            }
        } else {
            None
        };
        let fetched = execute::fetch(&self.state, task, cursor, resource).await?;
        let mut success = fetched.success;
        success.follows = open_follows(&self.strata, fetched.follows, fetched.max_fetches).await;
        Ok(success)
    }
}

async fn open_follows(
    strata: &Strata,
    works: Vec<FollowWork>,
    max_fetches: usize,
) -> Vec<strata_model::AccessTask> {
    let mut out = Vec::new();
    for work in works {
        if out.len() >= max_fetches {
            break;
        }
        if let Some(rid) = strata.resolve(&work.identity).await.ok().flatten() {
            let reference = FacetRef::new(
                rid,
                FacetKind::Struct,
                Some("page".into()),
                VersionSel::Latest,
            );
            if let Ok(reference) = reference {
                if let Ok(Some(facet)) = strata.get(&reference, true).await {
                    if needs_met(facet.value.as_ref(), &work.needs) {
                        continue;
                    }
                }
            }
        }
        out.push(work.task);
    }
    out
}

fn needs_met(value: Option<&Value>, needs: &[String]) -> bool {
    let Some(value) = value else {
        return false;
    };
    needs.iter().all(|need| {
        value.get(need).is_some_and(|item| match item {
            Value::String(text) => !text.is_empty(),
            Value::Null => false,
            _ => true,
        })
    })
}

struct PaidReceipts {
    paid: Arc<PaidGw>,
}

#[async_trait]
impl Receipts for PaidReceipts {
    async fn complete(&self, receipt_id: &str) -> Result<(), planner::PlannerError> {
        self.paid
            .complete(receipt_id)
            .await
            .map_err(|error| planner::PlannerError::with_source(error.message.clone(), error))
    }
}

fn normalize(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}
