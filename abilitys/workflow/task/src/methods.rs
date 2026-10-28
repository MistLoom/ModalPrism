//! 工具门面：`task.project_set` / `submit` / `status` / `next` / `get` / `update`
//! 仅调`task-support`，不感知 cli/mcp 等端口

use std::sync::Arc;

use crate::support::{
    TaskDraft, TaskError, TaskHub, TaskStatus, graph_summary, project_to_value, task_to_value,
    tasks_to_value, wrap,
};
use ability_support::{Error, ErrorKind};
use serde::Deserialize;
use serde_json::{Value, json};

fn map_err(e: TaskError) -> Error {
    let kind = match &e {
        TaskError::Invalid(_) => ErrorKind::InvalidInput,
        TaskError::NotFound(_) => ErrorKind::NotFound,
        TaskError::Conflict(_) => ErrorKind::Failed,
        TaskError::Version(_) => ErrorKind::Config,
        TaskError::Io(_) => ErrorKind::Io,
    };
    Error::from_ext(kind, e)
}

fn parse_status(s: &str) -> Result<TaskStatus, Error> {
    match s {
        "unclaimed" => Ok(TaskStatus::Unclaimed),
        "claimed" => Ok(TaskStatus::Claimed),
        "running" => Ok(TaskStatus::Running),
        "verifying" => Ok(TaskStatus::Verifying),
        "completed" => Ok(TaskStatus::Completed),
        "failed" => Ok(TaskStatus::Failed),
        other => Err(Error::invalid_args(format!("非法 status: {other}"))),
    }
}

pub(crate) struct ProjectSetTool {
    pub(crate) hub: Arc<TaskHub>,
}
#[derive(Deserialize)]
pub(crate) struct ProjectSetArgs {
    project_id: String,
    format: Option<String>,
    data_dir: Option<String>,
    default_format: Option<String>,
}
impl ProjectSetTool {
    pub async fn invoke(&self, args: Value) -> Result<Value, Error> {
        let a: ProjectSetArgs =
            serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
        self.hub
            .with_project(&a.project_id, |s| {
                let meta = s.project_set(a.data_dir, a.default_format)?.clone();
                let fmt = s.resolve_format(a.format.as_deref())?;
                let payload = json!({ "project": project_to_value(&meta) });
                Ok(wrap(fmt, payload))
            })
            .await
            .map_err(map_err)
    }
}

pub(crate) struct SubmitTool {
    pub(crate) hub: Arc<TaskHub>,
}
#[derive(Deserialize)]
pub(crate) struct SubmitArgs {
    project_id: String,
    format: Option<String>,
    tasks: Vec<TaskDraft>,
}
impl SubmitTool {
    pub async fn invoke(&self, args: Value) -> Result<Value, Error> {
        let a: SubmitArgs =
            serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
        self.hub
            .with_project(&a.project_id, |s| {
                let submitted = s.submit(a.tasks)?;
                let fmt = s.resolve_format(a.format.as_deref())?;
                let refs: Vec<_> = submitted.iter().collect();
                Ok(wrap(fmt, json!({ "tasks": tasks_to_value(&refs) })))
            })
            .await
            .map_err(map_err)
    }
}

pub(crate) struct StatusTool {
    pub(crate) hub: Arc<TaskHub>,
}
#[derive(Deserialize)]
pub(crate) struct StatusArgs {
    project_id: String,
    format: Option<String>,
    task_id: Option<String>,
}
impl StatusTool {
    pub async fn invoke(&self, args: Value) -> Result<Value, Error> {
        let a: StatusArgs =
            serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
        self.hub
            .with_project(&a.project_id, |s| {
                let fmt = s.resolve_format(a.format.as_deref())?;
                let payload = if let Some(tid) = a.task_id {
                    let t = s
                        .graph()
                        .tasks
                        .get(&tid)
                        .ok_or_else(|| TaskError::NotFound(format!("任务不存 {tid}")))?;
                    json!({ "task": task_to_value(t) })
                } else {
                    json!({
                        "project": project_to_value(s.meta()),
                        "summary": graph_summary(s.graph()),
                    })
                };
                Ok(wrap(fmt, payload))
            })
            .await
            .map_err(map_err)
    }
}

pub(crate) struct NextTool {
    pub(crate) hub: Arc<TaskHub>,
}
#[derive(Deserialize)]
pub(crate) struct NextArgs {
    project_id: String,
    format: Option<String>,
    claimed_by: String,
}
impl NextTool {
    pub async fn invoke(&self, args: Value) -> Result<Value, Error> {
        let a: NextArgs =
            serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
        self.hub
            .with_project(&a.project_id, |s| {
                let fmt = s.resolve_format(a.format.as_deref())?;
                match s.claim_next(&a.claimed_by)? {
                    Some(t) => Ok(wrap(
                        fmt,
                        json!({ "task": task_to_value(&t), "empty": false }),
                    )),
                    None => Ok(wrap(fmt, json!({ "task": Value::Null, "empty": true }))),
                }
            })
            .await
            .map_err(map_err)
    }
}

pub(crate) struct GetTool {
    pub(crate) hub: Arc<TaskHub>,
}
#[derive(Deserialize)]
pub(crate) struct GetArgs {
    project_id: String,
    format: Option<String>,
    mode: String,
}
impl GetTool {
    pub async fn invoke(&self, args: Value) -> Result<Value, Error> {
        let a: GetArgs =
            serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
        self.hub
            .with_project(&a.project_id, |s| {
                let fmt = s.resolve_format(a.format.as_deref())?;
                let tasks = match a.mode.as_str() {
                    "claimable" => {
                        let v = s.graph().claimable_tasks();
                        tasks_to_value(&v)
                    }
                    "all" => {
                        let mut ids: Vec<_> = s.graph().tasks.keys().cloned().collect();
                        ids.sort();
                        let refs: Vec<_> = ids
                            .iter()
                            .filter_map(|id| s.graph().tasks.get(id))
                            .collect();
                        tasks_to_value(&refs)
                    }
                    other => {
                        return Err(TaskError::Invalid(format!("非法 mode: {other}")));
                    }
                };
                Ok(wrap(fmt, json!({ "tasks": tasks, "mode": a.mode })))
            })
            .await
            .map_err(map_err)
    }
}

pub(crate) struct UpdateTool {
    pub(crate) hub: Arc<TaskHub>,
}
#[derive(Deserialize)]
pub(crate) struct UpdateArgs {
    project_id: String,
    format: Option<String>,
    task_id: String,
    status: String,
    claimed_by: Option<String>,
}
impl UpdateTool {
    pub async fn invoke(&self, args: Value) -> Result<Value, Error> {
        let a: UpdateArgs =
            serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
        let status = parse_status(&a.status)?;
        self.hub
            .with_project(&a.project_id, |s| {
                let t = s.update(&a.task_id, status, a.claimed_by.as_deref())?;
                let fmt = s.resolve_format(a.format.as_deref())?;
                Ok(wrap(fmt, json!({ "task": task_to_value(&t) })))
            })
            .await
            .map_err(map_err)
    }
}
