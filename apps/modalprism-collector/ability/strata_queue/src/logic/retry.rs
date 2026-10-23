//! 方法逻辑：retry

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
        return crate::invalid();
    };
    let retried = services.queue_retry(&task_id).await;
    crate::map(
        retried.map(
            |retried| serde_json::json!({ "taskId": task_id.to_string(), "retried": retried }),
        ),
    )
}
