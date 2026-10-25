//! 方法逻辑：list

use std::sync::Arc;

use assembly::domain::{QueueName, TaskFilter, known_reason, known_status};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Args {
    queue: Option<QueueName>,
    status: Option<String>,
    source: Option<String>,
    reason: Option<String>,
    query: Option<String>,
    before: Option<String>,
    cursor: Option<String>,
    limit: Option<u64>,
}

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let args: Args = crate::parse(args)?;
    let limit = args.limit.unwrap_or(60).clamp(1, 200) as usize;
    let status = non_empty(args.status);
    let reason = non_empty(args.reason);
    if let Some(status) = status.as_deref() {
        if !known_status(status) {
            return crate::invalid();
        }
    }
    if let Some(reason) = reason.as_deref() {
        if !known_reason(reason) {
            return crate::invalid();
        }
    }
    let filter = TaskFilter {
        queue: args.queue,
        status,
        source: non_empty(args.source),
        reason,
        query: non_empty(args.query),
        before: non_empty(args.before),
    };
    let cursor = non_empty(args.cursor);
    crate::map(services.queue_list(&filter, cursor.as_deref(), limit).await)
}

/// 空串一律当成没填，避免前端把下拉框的空值传进来。
fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.trim().is_empty())
}
