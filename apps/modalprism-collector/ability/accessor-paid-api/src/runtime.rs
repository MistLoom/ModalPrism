use ability_support::{Error, Result};
use collector_bind::BindConfig;
use serde_json::Value;

static BIND: std::sync::OnceLock<BindConfig> = std::sync::OnceLock::new();

pub async fn prime(config: Value) -> Result<()> {
    if BIND.get().is_some() {
        return Ok(());
    }
    let bind = BindConfig::parse(&config)?;
    BIND.set(bind).map_err(|_| Error::failed("accessor_paid_api 已初始化"))?;
    Ok(())
}

pub fn bind() -> Result<BindConfig> {
    BIND.get().cloned().ok_or_else(|| Error::failed("accessor_paid_api 未就绪"))
}
