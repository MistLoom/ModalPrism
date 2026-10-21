use std::sync::Arc;

use ability_support::Result;
use serde_json::Value;
use strata_client::Strata;
use strata_shared::{failed, text_field};

pub async fn run(strata: &Arc<Strata>, args: &Value) -> Result<Value> {
    let identity = text_field(args, "identity")?;
    let rid = strata
        .resolve(&identity)
        .await
        .map_err(|e| failed(e.to_string()))?;
    Ok(match rid {
        Some(rid) => Value::String(rid.to_string()),
        None => Value::Null,
    })
}
