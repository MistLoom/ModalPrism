//! 方法逻辑：enqueue

use std::sync::Arc;

use assembly::domain::AccessTask;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    tasks: Vec<AccessTask>,
}

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let args: Args = crate::parse(args)?;
    crate::unit(services.queue_enqueue(&args.tasks).await)
}
