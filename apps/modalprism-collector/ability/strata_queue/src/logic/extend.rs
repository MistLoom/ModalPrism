//! 方法逻辑：extend

use std::sync::Arc;
use std::time::Duration;

use assembly::domain::TaskId;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Args {
    task_id: String,
    lease_ms: u64,
}

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let args: Args = crate::parse(args)?;
    let Ok(task_id) = TaskId::new(args.task_id) else {
        return crate::invalid();
    };
    crate::unit(
        services
            .queue_extend(&task_id, Duration::from_millis(args.lease_ms))
            .await,
    )
}
