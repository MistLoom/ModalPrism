use std::sync::Arc;
use std::time::Duration;

use ability_support::{Error, Result};
use collector_bind::BindConfig;
use config::{ConfigStore, WatchHandle};
use serde_json::Value;
use tokio::sync::Mutex;

static RUNTIME: std::sync::OnceLock<Arc<Runtime>> = std::sync::OnceLock::new();

pub struct Runtime {
    pub store: Arc<ConfigStore>,
    pub bind: BindConfig,
    watch: Mutex<Option<WatchHandle>>,
}

pub async fn prime(config: Value) -> Result<()> {
    if RUNTIME.get().is_some() {
        return Ok(());
    }
    let bind = BindConfig::parse(&config)?;
    let store = ConfigStore::open(&bind.config_dir, bind.load_options())
        .map_err(|e| Error::failed(e.to_string()))?;
    let store = Arc::new(store);
    let handle = config::watch(Arc::clone(&store), Duration::from_secs(2))
        .map_err(|e| Error::failed(e.to_string()))?;
    let runtime = Arc::new(Runtime {
        store,
        bind,
        watch: Mutex::new(Some(handle)),
    });
    RUNTIME
        .set(runtime)
        .map_err(|_| Error::failed("config 已初始化"))?;
    Ok(())
}

pub fn get() -> Result<Arc<Runtime>> {
    RUNTIME
        .get()
        .cloned()
        .ok_or_else(|| Error::failed("config 未就绪"))
}
