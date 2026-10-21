use std::sync::Arc;

use ability_support::Result;
use serde::Deserialize;
use serde_json::Value;
use strata_client::Strata;
use strata_model::SpanRef;
use strata_shared::{failed, invalid_args, parse};

pub async fn run(strata: &Arc<Strata>, args: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        span: String,
    }
    let args: Args = parse(args)?;
    let span = SpanRef::parse(&args.span).map_err(|_| invalid_args("spec 无效"))?;
    let Some(facet) = strata
        .get(&span.facet, true)
        .await
        .map_err(|e| failed(e.to_string()))?
    else {
        return Err(failed("gateway"));
    };
    if facet.text.is_none() {
        return Err(failed("gateway"));
    }
    let bytes = strata.slice(&span).await.map_err(|_| failed("gateway"))?;
    let text = String::from_utf8(bytes.to_vec()).map_err(|_| failed("gateway"))?;
    Ok(Value::String(text))
}

pub async fn slice(ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    let _ = ctx;
    let args = strata_shared::normalize_args(args)?;
    run(&crate::runtime::strata()?, &args).await
}
