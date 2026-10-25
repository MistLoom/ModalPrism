//! 方法逻辑：sweep —— 保留策略清理；带筛选时兼任终态任务的清理入口。

use std::sync::Arc;

use assembly::domain::{QueueName, TaskFilter, known_reason, known_status};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Args {
    dry_run: Option<bool>,
    status: Option<String>,
    source: Option<String>,
    queue: Option<QueueName>,
    reason: Option<String>,
    query: Option<String>,
    before: Option<String>,
}

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let args: Args = crate::parse(args)?;
    let dry_run = args.dry_run.unwrap_or(true);
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
    let has_filter = status.is_some()
        || reason.is_some()
        || args.queue.is_some()
        || non_empty(args.source.clone()).is_some()
        || non_empty(args.query.clone()).is_some()
        || non_empty(args.before.clone()).is_some();
    let filter = has_filter.then(|| TaskFilter {
        queue: args.queue,
        status,
        source: non_empty(args.source),
        reason,
        query: non_empty(args.query),
        before: non_empty(args.before),
    });
    crate::map(services.sweep(dry_run, filter).await)
}

/// 空串一律当成没填。
fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.trim().is_empty())
}
