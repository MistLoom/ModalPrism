use std::path::Path;
use std::sync::Arc;

use ability_support::Result;
use serde_json::Value;
use strata_client::Strata;
use strata_shared::failed;

use super::status::status_paths;

pub async fn run(data_dir: &Path, strata: &Arc<Strata>, _args: &Value) -> Result<Value> {
    strata.recover().await.map_err(|e| failed(e.to_string()))?;
    Ok(status_paths(data_dir))
}

pub async fn maintain(ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
    let _ = ctx;
    let args = strata_shared::normalize_args(args)?;
    run(&crate::runtime::data_dir()?, &crate::runtime::strata()?, &args).await
}
