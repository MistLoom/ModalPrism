use std::sync::Arc;

use ability_support::Result;
use serde::Deserialize;
use serde_json::{Value, json};
use strata_client::Strata;
use strata_shared::{failed, parse};

pub async fn run(strata: &Arc<Strata>, args: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        after: u64,
        #[serde(default)]
        limit: Option<u32>,
    }
    let args: Args = parse(args)?;
    let rows = strata
        .changes(args.after, args.limit)
        .await
        .map_err(|e| failed(e.to_string()))?;
    let items = rows
        .into_iter()
        .map(|row| {
            json!({
                "snapshot": row.snapshot,
                "commitIds": row.commit_ids,
                "publishedAt": row.published_at,
            })
        })
        .collect();
    Ok(Value::Array(items))
}
