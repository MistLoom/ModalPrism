use std::path::PathBuf;
use std::sync::Arc;

use ability_support::{Error, Result};
use serde_json::Value;
use strata_client::Strata;
use strata_shared::{data_dir_of, open_strata};

static RUNTIME: std::sync::OnceLock<(PathBuf, Arc<Strata>)> = std::sync::OnceLock::new();

pub async fn prime(config: Value) -> Result<()> {
    if RUNTIME.get().is_some() {
        return Ok(());
    }
    let data_dir = data_dir_of(&config)?;
    let strata = open_strata(&config).await?;
    RUNTIME
        .set((data_dir, strata))
        .map_err(|_| Error::failed("committer 已初始化"))?;
    Ok(())
}

pub fn data_dir() -> Result<PathBuf> {
    RUNTIME
        .get()
        .map(|(dir, _)| dir.clone())
        .ok_or_else(|| Error::failed("committer 未就绪"))
}

pub fn strata() -> Result<Arc<Strata>> {
    RUNTIME
        .get()
        .map(|(_, strata)| Arc::clone(strata))
        .ok_or_else(|| Error::failed("committer 未就绪"))
}
