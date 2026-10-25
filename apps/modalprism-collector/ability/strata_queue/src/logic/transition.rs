//! 方法逻辑：transition —— 任务生命周期动作的唯一入口，单条与批量走同一状态机约束。

use std::sync::Arc;

use assembly::domain::TaskId;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Args {
    task_ids: Vec<String>,
    action: String,
}

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let args: Args = crate::parse(args)?;
    if args.task_ids.is_empty() {
        return crate::invalid();
    }
    let mut done = 0u64;
    let mut skipped = 0u64;
    for raw in &args.task_ids {
        let Ok(task_id) = TaskId::new(raw) else {
            skipped += 1;
            continue;
        };
        let changed = match args.action.as_str() {
            "cancel" => services.queue_cancel(&task_id).await,
            "requeue" => services.queue_requeue(&task_id).await,
            "remove" => services.queue_remove(&task_id).await,
            _ => return crate::invalid(),
        };
        match changed {
            Ok(true) => done += 1,
            _ => skipped += 1,
        }
    }
    crate::map(Ok(serde_json::json!({
        "done": done,
        "skipped": skipped,
        "action": args.action,
    })))
}
