//! 方法逻辑：pause

use std::sync::Arc;

use assembly::domain::QueueName;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Args {
    queue: QueueName,
    paused: bool,
}

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let args: Args = crate::parse(args)?;
    services
        .queue_pause(args.queue, args.paused)
        .await
        .map_err(crate::failure)?;
    crate::map(services.queue_states().await)
}
