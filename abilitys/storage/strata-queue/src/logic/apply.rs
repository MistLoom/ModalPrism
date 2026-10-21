use std::sync::Arc;

use ability_support::Result;
use serde::Deserialize;
use serde_json::Value;
use strata_client::Strata;
use strata_model::{ApplyOpts, ChangeSet};
use strata_shared::{failed, parse};

pub async fn run(strata: &Arc<Strata>, args: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        changeset: ChangeSet,
    }
    let args: Args = parse(args)?;
    strata
        .apply(args.changeset, ApplyOpts::default())
        .await
        .map_err(|e| failed(e.to_string()))?;
    Ok(Value::Null)
}
