//! 方法逻辑：validate

use std::sync::Arc;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    implementation: String,
    spec: serde_json::Value,
}

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let args: Args = crate::parse(args)?;
    crate::unit(services.accessor_validate(&args.implementation, &args.spec))
}
