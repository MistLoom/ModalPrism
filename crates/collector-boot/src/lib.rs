//! 组合根持有的运行时资源（存储、配置、出站代理）。不是 LEAF Gateway。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use collector_gateway::{HttpTransport, SystemResolver, call_policy_from, policy_of};
use egress::TcpDial;
use collector_runtime::{
    Binds, Error, PushCommit, execute, refuse_remote, require_loopback,
};
use collector_runtime::stage::DiskStaging;
use config::{ConfigStore, LoadOptions};
use serde_json::Value;
use strata_client::Strata;
use strata_model::{ApplyOpts, ChangeSet, Producer, RawEnvelope};
use strata_shared::strata_options;

use crate::bridges::{EgressGw, PaidBridge, PaidGw, PolicyEgress, PolicyPaid};
use crate::hot::Hot;

pub use collector_gateway::{call_policy_from as call_policy, policy_from};
pub use collector_runtime::plans::{capacities, flush_interval, snapshot_interval};

mod bridges;
mod hot;
pub mod planner_support;

pub struct BootResources {
    data_dir: PathBuf,
    config_dir: PathBuf,
    config: Arc<ConfigStore>,
    strata: Arc<Strata>,
    planner: planner::Planner<planner_support::StrataQueue, planner_support::PlannerPersist>,
    egress: Arc<EgressGw>,
    paid: Arc<PaidGw>,
    proxy: Mutex<String>,
    push: accessors_builtin::PushAccessor,
    browser_program: Mutex<PathBuf>,
}

impl BootResources {
    pub async fn open(data: &Path, config_dir: &Path, binds: Binds) -> Result<Arc<Self>, Error> {
        let load_options = LoadOptions::from_process();
        let snapshot = config::validate(config_dir, &load_options)?;
        refuse_remote(&snapshot, data)?;
        require_loopback(binds.http)?;
        require_loopback(binds.proxy)?;
        require_loopback(binds.push)?;
        let config = Arc::new(ConfigStore::open(config_dir, load_options)?);
        let opening = policy_of(&snapshot.system);
        let strata = Arc::new(Strata::open(data, strata_options(&snapshot.system)).await?);
        let planner = planner::Planner::new(
            planner_support::StrataQueue(Arc::clone(&strata)),
            planner_support::PlannerPersist(Arc::clone(&strata)),
        );
        let egress = Arc::new(EgressGw::new(
            Hot::new(Arc::clone(&strata), "egress"),
            HttpTransport,
            SystemResolver,
            TcpDial,
            opening.clone(),
        ));
        let paid = Arc::new(PaidGw::new(
            Hot::new(Arc::clone(&strata), "paid"),
            PaidBridge {
                egress: Arc::clone(&egress),
                fallback: opening,
            },
            call_policy_from(&snapshot.system, &Value::Null),
        ));
        let browser = std::env::var("MODALPRISM_MOLI").unwrap_or_else(|_| "moli".into());
        Ok(Arc::new(Self {
            data_dir: collector_runtime::normalize(data),
            config_dir: config_dir.to_path_buf(),
            config,
            strata,
            planner,
            egress,
            paid,
            proxy: Mutex::new(format!("http://{}", binds.proxy)),
            push: accessors_builtin::PushAccessor::new(),
            browser_program: Mutex::new(PathBuf::from(browser)),
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

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn set_proxy(&self, url: impl Into<String>) {
        *self.proxy.lock().expect("代理") = url.into();
    }

    pub fn proxy_url(&self) -> String {
        self.proxy.lock().expect("代理").clone()
    }

    pub fn browser_program(&self) -> PathBuf {
        self.browser_program.lock().expect("浏览器").clone()
    }

    pub fn egress_gateway(&self) -> Arc<EgressGw> {
        Arc::clone(&self.egress)
    }

    pub fn planner(&self) -> &planner::Planner<planner_support::StrataQueue, planner_support::PlannerPersist> {
        &self.planner
    }

    pub async fn shutdown(&self) -> Result<(), Error> {
        self.planner
            .shutdown(strata_model::Timestamp::now())
            .await
            .map_err(|e| Error::new(e.to_string()))?;
        Ok(())
    }

    pub fn fetch_env(&self, timeout: Duration) -> execute::FetchEnv {
        execute::FetchEnv {
            snapshot: self.config.applied(),
            staging_dir: self.data_dir.join("staging"),
            proxy: self.proxy_url(),
            browser: self.browser_program(),
            timeout,
        }
    }

    pub fn access_context(
        &self,
        egress_policy: egress::Policy,
        paid_policy: paid::CallPolicy,
    ) -> accessors::AccessContext {
        use collector_runtime::execute::NoSession;
        accessors::AccessContext {
            egress: Arc::new(PolicyEgress {
                inner: Arc::clone(&self.egress),
                policy: egress_policy,
            }),
            paid: Arc::new(PolicyPaid {
                inner: Arc::clone(&self.paid),
                policy: paid_policy,
            }),
            sessions: Arc::new(NoSession),
            staging: Arc::new(DiskStaging {
                dir: self.data_dir.join("staging"),
            }),
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    fn push(&self) -> &accessors_builtin::PushAccessor {
        &self.push
    }

    pub async fn commit_inbound(
        &self,
        source_id: &str,
        envelopes: &[RawEnvelope],
    ) -> Result<(), Error> {
        let snapshot = self.config.applied();
        let module = snapshot
            .modules
            .get(source_id)
            .cloned()
            .ok_or_else(|| Error::new("config"))?;
        let env = self.fetch_env(Duration::from_secs(30));
        let ops = execute::inbound_ops(&env, source_id, &module, envelopes)
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
}

#[async_trait]
impl PushCommit for BootResources {
    fn applied_snapshot(&self) -> Arc<config::Snapshot> {
        self.config.applied()
    }

    fn access_context(&self) -> accessors::AccessContext {
        let snap = self.config.applied();
        let policy = policy_of(&snap.system);
        let paid = call_policy_from(&snap.system, &Value::Null);
        self.access_context(policy, paid)
    }

    fn push_accessor(&self) -> &accessors_builtin::PushAccessor {
        self.push()
    }

    async fn commit_inbound(
        &self,
        source_id: &str,
        envelopes: &[RawEnvelope],
    ) -> Result<(), Error> {
        BootResources::commit_inbound(self, source_id, envelopes).await
    }
}

pub async fn serve_egress_proxy<F>(
    gateway: Arc<EgressGw>,
    listener: tokio::net::TcpListener,
    policy_of: F,
) where
    F: Fn() -> egress::Policy + Send + Sync + 'static,
{
    egress::serve_each(&gateway, listener, policy_of).await;
}
