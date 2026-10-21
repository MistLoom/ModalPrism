use std::collections::BTreeMap;
use std::sync::Arc;

use config::Snapshot;
use serde::Deserialize;
use serde_json::Value;

use crate::call::CtxCall;

#[derive(Deserialize)]
struct Applied {
    generation: u64,
    system: Value,
    modules: BTreeMap<String, Value>,
}

pub async fn applied(ctx: &dyn CtxCall) -> Result<Arc<Snapshot>, String> {
    let value = ctx.call("config", "applied", serde_json::json!({})).await?;
    let applied: Applied =
        serde_json::from_value(value).map_err(|_| "spec 无效".to_string())?;
    Ok(Arc::new(Snapshot {
        generation: applied.generation,
        system: applied.system,
        modules: applied.modules,
        origins: BTreeMap::new(),
    }))
}
