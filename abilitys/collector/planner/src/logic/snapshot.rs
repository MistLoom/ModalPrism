use std::collections::BTreeMap;
use std::sync::Arc;

use ability_support::{AbilityCtx, Error, Result};
use config::Snapshot;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct Applied {
    generation: u64,
    system: Value,
    modules: BTreeMap<String, Value>,
}

pub async fn from_ctx(ctx: &dyn AbilityCtx) -> Result<Arc<Snapshot>> {
    let applied_method = ability_support::MethodId::new("applied").expect("method id");
    let value = ctx.call("config", &applied_method, json!({})).await?;
    let applied: Applied =
        serde_json::from_value(value).map_err(|_| Error::invalid_args("spec 无效"))?;
    Ok(Arc::new(Snapshot {
        generation: applied.generation,
        system: applied.system,
        modules: applied.modules,
        origins: BTreeMap::new(),
    }))
}
