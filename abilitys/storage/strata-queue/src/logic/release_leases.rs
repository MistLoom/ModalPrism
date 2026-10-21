use std::sync::Arc;

use ability_support::Result;
use serde_json::Value;
use strata_client::Strata;
use strata_shared::failed;

pub async fn run(strata: &Arc<Strata>, _args: &Value) -> Result<Value> {
    let tasks = strata
        .release_leases()
        .await
        .map_err(|e| failed(e.to_string()))?;
    serde_json::to_value(tasks).map_err(|e| failed(e.to_string()))
}
