//! 方法逻辑：nack

use std::sync::Arc;

use assembly::domain::{TaskId, Timestamp};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Args {
    task_id: String,
    retry_at: String,
    error: String,
}

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let args: Args = crate::parse(args)?;
    let (Ok(task_id), Ok(retry_at)) = (TaskId::new(args.task_id), Timestamp::parse(&args.retry_at))
    else {
        return crate::invalid();
    };
    crate::unit(services.queue_nack(&task_id, retry_at, &args.error).await)
}
