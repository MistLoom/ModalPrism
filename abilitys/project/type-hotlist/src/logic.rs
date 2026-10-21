use ability_support::{AbilityCtx, Error, MethodId, Result};
use serde_json::{Value, json};
use strata_client::Strata;
use strata_model::Rid;

pub async fn project(
    _ctx: &dyn AbilityCtx,
    _strata: &Strata,
    args: Value,
) -> Result<Value> {
    let rid = args
        .get("rid")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::invalid_args("spec 无效"))?;
    let _ = Rid::parse(rid).map_err(|_| Error::invalid_args("spec 无效"))?;
    Ok(Value::Null)
}

pub async fn on_commit(
    ctx: &dyn AbilityCtx,
    strata: &Strata,
    args: Value,
) -> Result<Value> {
    if args.as_object().is_some_and(|object| !object.is_empty()) {
        return Err(Error::invalid_args("spec 无效"));
    }
    let applied = ctx
        .call(
            "config",
            &MethodId::new("applied").expect("method id"),
            json!({}),
        )
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
    types::HotList::on_commit(strata, &snapshot)
        .await
        .map_err(|e| Error::failed(e.to_string()))
}
