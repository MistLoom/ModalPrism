use std::sync::Arc;

use ability_support::Result;
use serde::Deserialize;
use serde_json::Value;
use strata_client::Strata;
use strata_model::{FacetKind, Rid};
use strata_shared::{failed, invalid_args, parse};

pub async fn run(strata: &Arc<Strata>, args: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        rid: String,
        kind: FacetKind,
        #[serde(default)]
        variant: Option<String>,
    }
    let args: Args = parse(args)?;
    let rid = Rid::parse(&args.rid).map_err(|_| invalid_args("spec 无效"))?;
    let found = strata
        .has(&rid, &args.kind, args.variant.as_deref())
        .await
        .map_err(|e| failed(e.to_string()))?;
    Ok(Value::Bool(found))
}

pub async fn has(ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    let _ = ctx;
    let args = strata_shared::normalize_args(args)?;
    run(&crate::runtime::strata()?, &args).await
}
