use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec, Result};
use collector_boot::{BootResources, snapshot_interval};
use serde_json::{Value, json};
use strata_model::{ApplyOpts, ChangeSet, Producer, Timestamp};

pub fn provider(services: Arc<BootResources>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("health").stateful();
    spec = spec.on(
        "evaluate",
        "把健康写入存储",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            move |_ctx, _args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(async move { evaluate(&services).await })
            }
        },
    );
    spec = spec.on(
        "status",
        "读取信源健康",
        serde_json::json!({"type": "object"}),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(async move { status(&services, args).await })
            }
        },
    );
    spec.into_provider()
}

async fn evaluate(services: &BootResources) -> Result<Value> {
    let now = Timestamp::now();
    let applied = services.config().applied();
    let ops = services
        .planner()
        .snapshot(now, snapshot_interval(&applied))
        .await
        .map_err(|e| ability_support::Error::failed(e.to_string()))?;
    if !ops.is_empty() {
        let changeset = ChangeSet {
            producer: Producer {
                module: "health".into(),
                version: "1".into(),
                config_generation: applied.generation,
            },
            idempotency_key: None,
            ack: None,
            ops,
        };
        services
            .strata()
            .apply(changeset, ApplyOpts::default())
            .await
            .map_err(|e| ability_support::Error::failed(e.to_string()))?;
        services
            .planner()
            .commit_snapshot(now)
            .await
            .map_err(|e| ability_support::Error::failed(e.to_string()))?;
    }
    Ok(json!({}))
}

async fn status(services: &BootResources, args: Value) -> Result<Value> {
    let source = args
        .get("source")
        .and_then(Value::as_str)
        .ok_or_else(|| ability_support::Error::invalid_args("spec 无效"))?;
    let status = services
        .planner()
        .source_status(source)
        .await
        .map_err(|e| ability_support::Error::failed(e.to_string()))?;
    Ok(json!({
        "key": format!("source/{source}"),
        "status": status.health_status,
        "alert": status.alert,
        "lastError": status.last_error,
        "lastYieldAt": status.last_yield_at,
    }))
}
