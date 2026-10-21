use std::sync::Arc;

use ability_support::Result;
use serde::Deserialize;
use serde_json::{Value, json};
use strata_client::Strata;
use strata_model::{EdgeDir, Rid};
use strata_shared::{failed, invalid_args, parse};

pub async fn run(strata: &Arc<Strata>, args: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        rid: String,
        #[serde(default)]
        rel: Option<String>,
        #[serde(default = "out_dir")]
        direction: String,
    }
    fn out_dir() -> String {
        "out".into()
    }
    let args: Args = parse(args)?;
    let direction = match args.direction.as_str() {
        "out" => EdgeDir::Out,
        "in" => EdgeDir::In,
        _ => return Err(invalid_args("spec 无效")),
    };
    let rid = Rid::parse(&args.rid).map_err(|_| invalid_args("spec 无效"))?;
    let rows = strata
        .related(&rid, args.rel.as_deref(), direction)
        .await
        .map_err(|e| failed(e.to_string()))?;
    let items = rows
        .into_iter()
        .map(|row| {
            json!({
                "src": row.src.to_string(),
                "rel": row.rel,
                "dst": row.dst.to_string(),
                "attrs": row.attrs,
                "commitId": row.commit_id,
            })
        })
        .collect();
    Ok(Value::Array(items))
}

pub async fn related(ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    let _ = ctx;
    let args = strata_shared::normalize_args(args)?;
    run(&crate::runtime::strata()?, &args).await
}
