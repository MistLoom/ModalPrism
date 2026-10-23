//! 方法逻辑：list

use std::sync::Arc;

use assembly::domain::QueueName;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Args {
    queue: Option<QueueName>,
    status: Option<String>,
    limit: Option<u64>,
}

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let args: Args = crate::parse(args)?;
    let limit = args.limit.unwrap_or(60).clamp(1, 200) as usize;
    let status = args.status.as_deref().filter(|text| !text.is_empty());
    crate::map(services.queue_list(args.queue, status, limit).await)
}
