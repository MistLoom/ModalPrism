//! 绑定期付费网关；须在 `egress` prime 之后、`load` 之前调用 `prime`。

use std::sync::Arc;

use ability_support::{Error, Result};
use collector_bind::BindConfig;
use collector_gateway::{call_policy_from, policy_of};
use serde_json::Value;
use strata_shared::attach_strata;

use crate::bridge::{PaidBridge, PaidGw};
use crate::hot::Hot;

pub struct Runtime {
    gateway: Arc<PaidGw>,
    system: Value,
}

static RUNTIME: std::sync::OnceLock<Arc<Runtime>> = std::sync::OnceLock::new();

pub async fn prime(config: Value) -> Result<()> {
    if RUNTIME.get().is_some() {
        return Ok(());
    }
    let bind = BindConfig::parse(&config)?;
    let strata = attach_strata(&bind.strata_config()).await?;
    let opening = policy_of(&bind.system);
    let egress = ability_egress::runtime::get()?.gateway();
    let gateway = Arc::new(paid::Gateway::new(
        Hot::new(strata, "paid"),
        PaidBridge {
            egress,
            fallback: opening,
        },
        call_policy_from(&bind.system, &Value::Null),
    ));
    let runtime = Arc::new(Runtime {
        gateway,
        system: bind.system,
    });
    RUNTIME
        .set(runtime)
        .map_err(|_| Error::failed("paid_gateway 已初始化"))?;
    Ok(())
}

pub fn get() -> Result<Arc<Runtime>> {
    RUNTIME
        .get()
        .cloned()
        .ok_or_else(|| Error::failed("paid_gateway 未就绪"))
}

impl Runtime {
    pub fn gateway(&self) -> Arc<PaidGw> {
        Arc::clone(&self.gateway)
    }

    pub fn system(&self) -> &Value {
        &self.system
    }
}
