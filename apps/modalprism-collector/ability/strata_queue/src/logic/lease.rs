//! 方法逻辑：lease

use std::sync::Arc;
use std::time::Duration;

use assembly::domain::QueueName;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Args {
    queue: QueueName,
    count: u64,
    lease_ms: u64,
}

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let args: Args = crate::parse(args)?;
    let Ok(count) = usize::try_from(args.count) else {
        return crate::invalid();
    };
    crate::map(
        services
            .queue_lease(args.queue, count, Duration::from_millis(args.lease_ms))
            .await,
    )
}
