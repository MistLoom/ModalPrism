use std::sync::Arc;

use ability_support::Result;
use serde_json::{Value, json};
use strata_client::Strata;
use strata_model::FacetRef;
use strata_shared::{failed, invalid_args, text_field};

pub async fn run(strata: &Arc<Strata>, args: &Value) -> Result<Value> {
    let facet = text_field(args, "facet")?;
    let reference = FacetRef::parse(&facet).map_err(|_| invalid_args("spec 无效"))?;
    let versions = strata
        .history(&reference)
        .await
        .map_err(|e| failed(e.to_string()))?;
    let items: Vec<Value> = versions
        .into_iter()
        .map(|item| {
            json!({
                "version": item.version,
                "contentHash": item.content_hash,
                "createdAt": item.created_at,
                "seq": item.seq,
            })
        })
        .collect();
    Ok(Value::Array(items))
}

pub async fn history(ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    let _ = ctx;
    let args = strata_shared::normalize_args(args)?;
    run(&crate::runtime::strata()?, &args).await
}
