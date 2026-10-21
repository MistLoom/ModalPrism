use ability_support::{AbilityCtx, Error, MethodId, Result};
use serde_json::{Value, json};
use strata_client::Strata;
use strata_model::ApplyOpts;

pub async fn run(strata: &Strata, ctx: &dyn AbilityCtx) -> Result<serde_json::Value> {
    let applied = ctx
        .call("config", &MethodId::new("applied").unwrap(), json!({}))
        .await?;
    let generation = applied.get("generation").and_then(|v| v.as_u64()).unwrap_or(1);
    let system = applied.get("system").cloned().unwrap_or(json!({}));
    let modules: std::collections::BTreeMap<String, Value> = applied
        .get("modules")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let snapshot = config::Snapshot {
        generation,
        system,
        modules,
        origins: std::collections::BTreeMap::new(),
    };
    let value = types::HotList::on_commit(strata, &snapshot)
        .await
        .map_err(|e| Error::failed(e.to_string()))?;
    if value.is_null() {
        return Ok(json!({ "applied": false }));
    }
    let changeset = serde_json::from_value(value).map_err(|e| Error::failed(e.to_string()))?;
    strata
        .apply(changeset, ApplyOpts::default())
        .await
        .map_err(|e| Error::failed(e.to_string()))?;
    strata.flush().await.map_err(|e| Error::failed(e.to_string()))?;
    Ok(json!({ "applied": true }))
}
