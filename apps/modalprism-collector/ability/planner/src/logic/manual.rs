//! 方法逻辑：manual

use std::sync::Arc;

use assembly::domain::QueueName;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Args {
    source: String,
    queue: Option<QueueName>,
    run_now: Option<bool>,
    locator: Option<String>,
}

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let args: Args = crate::parse(args)?;
    let run_now = args.run_now.unwrap_or(false);
    crate::map(
        services
            .task_create(&args.source, args.queue, run_now, args.locator)
            .await,
    )
}
