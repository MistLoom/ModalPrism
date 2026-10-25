//! 方法逻辑：round

use std::sync::Arc;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Args {
    source: Option<String>,
}

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let args: Args = crate::parse(args)?;
    let source = args.source.filter(|text| !text.trim().is_empty());
    crate::map(services.dispatch_round(source.as_deref()).await)
}
