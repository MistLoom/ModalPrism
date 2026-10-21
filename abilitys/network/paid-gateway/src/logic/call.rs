use std::sync::Arc;

use ability_support::Result;
use collector_gateway::{call_policy_from, EGRESS_POLICY};
use paid::CallPolicy;
use serde::Deserialize;
use serde_json::{Value, json};
use strata_shared::{failed, parse};

use crate::bridge::PaidGw;

pub async fn run(
    gateway: &Arc<PaidGw>,
    system: &Value,
    args: &Value,
) -> Result<Value> {
    #[derive(Deserialize)]
    struct Args {
        endpoint: String,
        headers: Vec<(String, String)>,
        body: serde_json::Value,
        params: Value,
        policy: Option<Value>,
    }
    let args: Args = parse(args)?;
    let body = match args.body {
        Value::String(text) => text.into_bytes(),
        Value::Null => Vec::new(),
        other => serde_json::to_vec(&other).unwrap_or_default(),
    };
    let policy: CallPolicy = args
        .policy
        .map(|p| call_policy_from(system, &p))
        .unwrap_or_else(|| call_policy_from(system, &Value::Null));
    let response = gateway
        .call_governed(
            &policy,
            accessors::PaidCall {
                endpoint: args.endpoint,
                headers: args.headers,
                body: bytes::Bytes::from(body),
                params: args.params,
            },
        )
        .await
        .map_err(|e| failed(e.message))?;
    Ok(json!({
        "status": response.status,
        "headers": response.headers,
        "body": response.body.to_vec(),
        "receiptId": response.receipt_id,
    }))
}
