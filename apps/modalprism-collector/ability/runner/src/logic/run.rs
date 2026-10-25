//! 方法逻辑：run

use std::sync::Arc;

use assembly::domain::TaskId;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Args {
    task_id: String,
}

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let args: Args = crate::parse(args)?;
    let Ok(task_id) = TaskId::new(args.task_id) else {
        return Err(ability_support::Error::invalid_args("任务号无效"));
    };
    let outcome = services.run_task_by_id(&task_id).await;
    crate::map(outcome.map(|outcome| match outcome.nacked {
        Some(reason) => serde_json::json!({ "taskId": task_id.to_string(), "nacked": reason }),
        None => serde_json::json!({
            "taskId": task_id.to_string(),
            "ok": true,
            "follows": outcome.follows.len(),
        }),
    }))
}
