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
        owner: String,
        key: String,
    }
    let args: Args = parse(args)?;
    match strata
        .hot_get(&args.owner, &args.key)
        .await
        .map_err(|e| failed(e.to_string()))?
    {
        Some(value) => Ok(json!({ "value": value })),
        None => Ok(json!({ "value": null })),
    }
}

pub async fn load(ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    let _ = ctx;
    let args = strata_shared::normalize_args(args)?;
    run(&crate::runtime::strata()?, &args).await
}
