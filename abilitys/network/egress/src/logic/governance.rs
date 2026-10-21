use std::sync::Arc;

use ability_support::Result;
use serde_json::{Value, json};

use crate::runtime::EgressGw;

pub async fn domains(gateway: &Arc<EgressGw>, proxy_url: &str) -> Result<Value> {
    let db = gateway
        .egress_db()
        .await
        .map_err(|e| ability_support::Error::failed(e.message))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let domains: Vec<Value> = db
        .circuits
        .iter()
        .map(|(host, row)| {
            let open = egress::circuit_open(&db, host, now);
            let state_label = if open {
                "open"
            } else if row.open_until.is_some() {
                "half-open"
            } else {
                "closed"
            };
            json!({
                "host": host,
                "circuit": state_label,
                "consecutiveFailures": row.consecutive,
                "cooldownMs": row.cooldown_ms,
            })
        })
        .collect();
    Ok(json!({ "domains": domains, "proxy": proxy_url }))
}

pub async fn reset_circuit(gateway: &Arc<EgressGw>, host: &str) -> Result<Value> {
    gateway
        .reset_circuit(host)
        .await
        .map_err(|e| ability_support::Error::failed(e.message))?;
    Ok(json!({ "ok": true, "host": host }))
}
