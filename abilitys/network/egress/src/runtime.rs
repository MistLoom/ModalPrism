//! 绑定期配置与网关；装配在 load 前 `prime`。

use std::sync::Arc;

use ability_support::{Error, Result};
use collector_bind::BindConfig;
use collector_gateway::{HttpTransport, SystemResolver, policy_of};
use egress::{Resolver, TcpDial};
use serde_json::Value;
use strata_shared::attach_strata;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::hot::Hot;

pub type EgressGw = egress::Gateway<Hot, HttpTransport, SystemResolver, TcpDial>;

pub struct Runtime {
    gateway: Arc<EgressGw>,
    proxy_url: String,
    system: Value,
    proxy_listen: Option<String>,
    listener: Mutex<Option<JoinHandle<()>>>,
}

static RUNTIME: std::sync::OnceLock<Arc<Runtime>> = std::sync::OnceLock::new();

pub async fn prime(config: Value) -> Result<()> {
    if RUNTIME.get().is_some() {
        return Ok(());
    }
    let bind = BindConfig::parse(&config)?;
    let strata = attach_strata(&bind.strata_config()).await?;
    let opening = policy_of(&bind.system);
    let gateway = Arc::new(egress::Gateway::new(
        Hot::new(strata, "egress"),
        HttpTransport,
        SystemResolver,
        TcpDial,
        opening,
    ));
    let runtime = Arc::new(Runtime {
        gateway,
        proxy_url: bind.proxy_url.unwrap_or_default(),
        system: bind.system,
        proxy_listen: bind.proxy_listen,
        listener: Mutex::new(None),
    });
    if let Some(addr) = runtime.proxy_listen.as_deref() {
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| Error::failed(e.to_string()))?;
        let gw = Arc::clone(&runtime.gateway);
        let system = runtime.system.clone();
        let task = tokio::spawn(async move {
            egress::serve_each(&gw, listener, move || policy_of(&system)).await;
        });
        *runtime.listener.lock().await = Some(task);
    }
    RUNTIME
        .set(runtime)
        .map_err(|_| Error::failed("egress 已初始化"))?;
    Ok(())
}

pub fn get() -> Result<Arc<Runtime>> {
    RUNTIME
        .get()
        .cloned()
        .ok_or_else(|| Error::failed("egress 未就绪"))
}

impl Runtime {
    pub fn gateway(&self) -> Arc<EgressGw> {
        Arc::clone(&self.gateway)
    }

    pub fn proxy_url(&self) -> &str {
        &self.proxy_url
    }

    pub fn system(&self) -> &Value {
        &self.system
    }
}
