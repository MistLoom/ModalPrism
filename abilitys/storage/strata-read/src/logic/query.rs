use std::sync::Arc;

use ability_support::Result;
use serde::Deserialize;
use serde_json::{Value, json};
use strata_client::Strata;
use strata_model::Query;
use strata_shared::{failed, parse};

pub async fn run(strata: &Arc<Strata>, args: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        query: Query,
    }
    let args: Args = parse(args)?;
    let page = strata
        .query(args.query)
        .await
        .map_err(|e| failed(e.to_string()))?;
    Ok(json!({
        "rows": page.rows,
        "nextCursor": page.next_cursor,
    }))
}

pub async fn query(ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    let _ = ctx;
    let args = strata_shared::normalize_args(args)?;
    run(&crate::runtime::strata()?, &args).await
}
