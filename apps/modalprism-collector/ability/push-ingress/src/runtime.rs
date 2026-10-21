use std::sync::Arc;

use ability_support::{Error, Result};
use collector_bind::BindConfig;
use serde_json::Value;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

static BIND: std::sync::OnceLock<BindConfig> = std::sync::OnceLock::new();
static LISTENER: Mutex<Option<JoinHandle<()>>> = Mutex::const_new(None);

pub async fn prime(config: Value) -> Result<()> {
    if BIND.get().is_some() {
        return Ok(());
    }
    let bind = BindConfig::parse(&config)?;
    BIND.set(bind).map_err(|_| Error::failed("push_ingress 已初始化"))?;
    Ok(())
}

pub fn bind() -> Result<BindConfig> {
    BIND.get().cloned().ok_or_else(|| Error::failed("push_ingress 未就绪"))
}

pub async fn set_listener(task: JoinHandle<()>) {
    *LISTENER.lock().await = Some(task);
}

pub async fn stop_listener() {
    if let Some(task) = LISTENER.lock().await.take() {
        task.abort();
    }
}
