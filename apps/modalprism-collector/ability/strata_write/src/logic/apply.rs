//! 方法逻辑：apply

use std::sync::Arc;

use assembly::domain::ChangeSet;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    changeset: ChangeSet,
}

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let args: Args = crate::parse(args)?;
    crate::map(services.strata_apply(args.changeset).await)
}
