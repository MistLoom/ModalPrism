//! 方法逻辑：run_task

use std::sync::Arc;

use assembly::domain::AccessTask;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    task: AccessTask,
}

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let args: Args = crate::parse(args)?;
    let outcome = services.runner_run(args.task).await;
    crate::map(outcome.map(|outcome| match outcome.nacked {
        Some(reason) => serde_json::json!({ "nacked": reason }),
        None => serde_json::json!({ "ok": true, "follows": outcome.follows }),
    }))
}
