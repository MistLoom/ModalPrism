use std::sync::Arc;

use ability_support::{Error, Result};
use serde_json::Value;
use strata_client::Strata;
use strata_shared::attach_strata;

static STRATA: std::sync::OnceLock<Arc<Strata>> = std::sync::OnceLock::new();

pub async fn prime(config: Value) -> Result<()> {
    if STRATA.get().is_some() {
        return Ok(());
    }
    let strata = attach_strata(&config).await?;
    STRATA
        .set(strata)
        .map_err(|_| Error::failed("type_hotlist 已初始化"))?;
    Ok(())
}

pub fn strata() -> Result<Arc<Strata>> {
    STRATA
        .get()
        .cloned()
        .ok_or_else(|| Error::failed("type_hotlist 未就绪"))
}
