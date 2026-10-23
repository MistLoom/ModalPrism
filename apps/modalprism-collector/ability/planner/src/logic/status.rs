//! 方法逻辑：status

use std::sync::Arc;

use assembly::domain::SourceStatus;

/// 单条信源状态。`next`、`status` 与界面共用这一个投影。
pub fn project(status: &SourceStatus) -> serde_json::Value {
    serde_json::json!({
        "nextAt": status.next_at,
        "intervalMs": status.interval_ms,
        "healthStatus": status.health_status,
        "alert": status.alert,
        "skipReason": status.skip_reason,
        "budgetUsed": status.budget_used,
        "lastError": status.last_error,
        "lastOkAt": status.last_ok_at,
        "lastYieldAt": status.last_yield_at,
        "yields7d": status.yields_7d,
    })
}

pub async fn run(
    services: Arc<assembly::Services>,
    _args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let statuses = services.planner_statuses().await;
    crate::map(statuses.map(|rows| {
        rows.into_iter()
            .map(|(source, status)| {
                let mut item = project(&status);
                if let Some(object) = item.as_object_mut() {
                    object.insert("source".into(), serde_json::Value::String(source));
                }
                item
            })
            .collect::<Vec<_>>()
    }))
}
