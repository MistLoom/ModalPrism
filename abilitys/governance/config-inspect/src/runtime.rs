use std::path::PathBuf;
use std::sync::Arc;

use ability_support::{Error, Result};
use collector_bind::BindConfig;
use config::{ConfigStore, LoadOptions};
use serde_json::Value;

pub struct Runtime {
    pub config_dir: PathBuf,
    pub load_options: LoadOptions,
    pub store: Arc<ConfigStore>,
}

static RUNTIME: std::sync::OnceLock<Arc<Runtime>> = std::sync::OnceLock::new();

pub async fn prime(config: Value) -> Result<()> {
    if RUNTIME.get().is_some() {
        return Ok(());
    }
    let bind = BindConfig::parse(&config)?;
    let load_options = bind.load_options();
    let config_dir = bind.config_dir.clone();
    let store = ConfigStore::open(&bind.config_dir, load_options.clone())
        .map_err(|e| Error::failed(e.to_string()))?;
    RUNTIME
        .set(Arc::new(Runtime {
            config_dir,
            load_options,
            store: Arc::new(store),
        }))
        .map_err(|_| Error::failed("config_inspect 已初始化"))?;
    Ok(())
}

pub fn get() -> Result<Arc<Runtime>> {
    RUNTIME
        .get()
        .cloned()
        .ok_or_else(|| Error::failed("config_inspect 未就绪"))
}
