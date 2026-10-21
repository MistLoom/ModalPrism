use ability_support::{AbilityCtx, Error, MethodId, Result};
use serde_json::{Value, json};
use strata_model::{ChangeSet, Producer};

pub async fn evaluate(ctx: &dyn AbilityCtx, _args: Value) -> Result<Value> {
    if ctx.dep("strata_write").is_err() {
        return Err(Error::unavailable("health.evaluate 需要写入路径"));
    }
    let generation_payload = ctx
        .call("config", &MethodId::new("generation").unwrap(), json!({}))
        .await?;
    let generation = generation_payload
        .get("generation")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    let ops = ctx
        .call(
            "planner",
            &MethodId::new("health_snapshot").unwrap(),
            json!({}),
        )
        .await?;
    let ops = ops.as_array().cloned().unwrap_or_default();
    if ops.is_empty() {
        return Ok(json!({}));
    }
    let changeset = ChangeSet {
        producer: Producer {
            module: "health".into(),
            version: "1".into(),
            config_generation: generation,
        },
        idempotency_key: None,
        ack: None,
        ops: serde_json::from_value(Value::Array(ops))
            .map_err(|e| Error::failed(e.to_string()))?,
    };
    ctx.call(
        "strata_write",
        &MethodId::new("apply").unwrap(),
        json!({ "changeset": changeset }),
    )
    .await?;
    ctx.call(
        "planner",
        &MethodId::new("health_commit").unwrap(),
        json!({}),
    )
    .await?;
    Ok(json!({}))
}
