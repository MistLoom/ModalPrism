use std::sync::Arc;

use ability_support::Result;
use serde::Deserialize;
use serde_json::{Value, json};
use strata_client::Strata;
use strata_model::{FacetRef, LineageDir};
use strata_shared::{failed, invalid_args, parse};

pub async fn run(strata: &Arc<Strata>, args: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        facet: String,
        #[serde(default)]
        direction: Option<String>,
        #[serde(default = "one")]
        depth: u32,
    }
    fn one() -> u32 {
        1
    }
    let args: Args = parse(args)?;
    let direction = match args.direction.as_deref() {
        None | Some("up") => LineageDir::Up,
        Some("down") => LineageDir::Down,
        Some(_) => return Err(invalid_args("spec 无效")),
    };
    let reference = FacetRef::parse(&args.facet).map_err(|_| invalid_args("spec 无效"))?;
    let graph = strata
        .lineage(&reference, direction, args.depth)
        .await
        .map_err(|e| failed(e.to_string()))?;
    Ok(json!({
        "nodes": graph.nodes.iter().map(|node| json!({
            "facet": node.facet.to_string(),
            "module": node.module,
            "version": node.version,
            "paramsHash": node.params_hash,
        })).collect::<Vec<_>>(),
        "edges": graph.edges.iter().map(|edge| json!({
            "from": edge.from.to_string(),
            "to": edge.to.to_string(),
        })).collect::<Vec<_>>(),
    }))
}

pub async fn lineage(ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    let _ = ctx;
    let args = strata_shared::normalize_args(args)?;
    run(&crate::runtime::strata()?, &args).await
}
