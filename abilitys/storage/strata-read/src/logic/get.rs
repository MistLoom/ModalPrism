use std::sync::Arc;

use ability_support::Result;
use serde::Deserialize;
use serde_json::{Value, json};
use strata_client::Strata;
use strata_model::FacetRef;
use strata_shared::{failed, invalid_args, parse};

pub async fn run(strata: &Arc<Strata>, args: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        facet: String,
        #[serde(default = "lazy_default")]
        lazy: bool,
    }
    fn lazy_default() -> bool {
        true
    }
    let args: Args = parse(args)?;
    let reference = FacetRef::parse(&args.facet).map_err(|_| invalid_args("spec 无效"))?;
    let Some(facet) = strata
        .get(&reference, args.lazy)
        .await
        .map_err(|e| failed(e.to_string()))?
    else {
        return Ok(Value::Null);
    };
    Ok(json!({
        "version": facet.version,
        "contentHash": facet.content_hash,
        "mediaType": facet.media_type,
        "size": facet.size,
        "text": facet.text,
        "struct": facet.value,
        "part": facet.blob,
    }))
}

pub async fn get(ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    let _ = ctx;
    let args = strata_shared::normalize_args(args)?;
    run(&crate::runtime::strata()?, &args).await
}
