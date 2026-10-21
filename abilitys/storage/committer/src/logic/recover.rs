use std::sync::Arc;

use ability_support::Result;
use serde_json::{Value, json};
use strata_client::Strata;
use strata_shared::failed;

pub async fn run(strata: &Arc<Strata>, _args: &Value) -> Result<Value> {
    strata.recover().await.map_err(|e| failed(e.to_string()))?;
    Ok(json!({}))
}
