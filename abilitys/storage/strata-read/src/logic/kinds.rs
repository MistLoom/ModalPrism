use std::sync::Arc;

use ability_support::Result;
use serde::Deserialize;
use serde_json::Value;
use strata_client::Strata;
use strata_model::Rid;
use strata_shared::{failed, invalid_args, parse};

pub async fn run(strata: &Arc<Strata>, args: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        rid: String,
        #[serde(default)]
        include_variants: bool,
    }
    let args: Args = parse(args)?;
    let rid = Rid::parse(&args.rid).map_err(|_| invalid_args("spec 无效"))?;
    let kinds = strata
        .kinds(&rid, args.include_variants)
        .await
        .map_err(|e| failed(e.to_string()))?;
    serde_json::to_value(kinds).map_err(|e| failed(e.to_string()))
}

pub async fn kinds(ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    let _ = ctx;
    let args = strata_shared::normalize_args(args)?;
    run(&crate::runtime::strata()?, &args).await
}
