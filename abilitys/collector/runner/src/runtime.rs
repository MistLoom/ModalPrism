use std::sync::Arc;

use ability_support::{Error, Result};
use collector_bind::BindConfig;
use serde_json::Value;

use crate::env::RunnerEnv;

static RUNTIME: std::sync::OnceLock<Arc<RunnerEnv>> = std::sync::OnceLock::new();

pub async fn prime(config: Value) -> Result<()> {
    if RUNTIME.get().is_some() {
        return Ok(());
    }
    let bind = BindConfig::parse(&config)?;
    let env = Arc::new(RunnerEnv { bind });
    RUNTIME
        .set(env)
        .map_err(|_| Error::failed("runner 已初始化"))?;
    Ok(())
}

pub fn env() -> Result<Arc<RunnerEnv>> {
    RUNTIME
        .get()
        .cloned()
        .ok_or_else(|| Error::failed("runner 未就绪"))
}
