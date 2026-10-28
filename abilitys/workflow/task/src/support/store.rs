//! 工程落盘与认领。经 `store.persist`，不接触路径。

use std::sync::Arc;

use ability_store::{AcquireArgs, GetArgs, PutArgs, ReleaseArgs};
use ability_store_persist::PersistExt;
use ability_support::AbilityCtx;
use serde::Deserialize;
use tokio::sync::RwLock;

use super::error::TaskError;
use super::model::{
    OutputFormat, ProjectMeta, TASK_SYSTEM_VERSION, Task, TaskGraph, TaskStatus, can_transition,
    ensure_no_dup_deps,
};

const OWNER: &str = "task";

/// 多工程 Hub。
#[derive(Clone)]
pub struct TaskHub {
    ctx: Arc<RwLock<Option<Arc<dyn AbilityCtx>>>>,
}

impl TaskHub {
    pub fn new() -> Self {
        Self {
            ctx: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn bind_ctx(&self, ctx: Arc<dyn AbilityCtx>) {
        *self.ctx.write().await = Some(ctx);
    }

    async fn ctx(&self) -> Result<Arc<dyn AbilityCtx>, TaskError> {
        self.ctx
            .read()
            .await
            .clone()
            .ok_or_else(|| TaskError::Io("store.persist 未绑定".into()))
    }

    pub async fn with_project<R>(
        &self,
        project_id: &str,
        f: impl FnOnce(&mut ProjectStore) -> Result<R, TaskError>,
    ) -> Result<R, TaskError> {
        validate_project_id(project_id)?;
        let ctx = self.ctx().await?;
        let token = ctx
            .persist()
            .acquire(AcquireArgs {
                owner: Some(OWNER.into()),
                space: None,
                key: format!("{project_id}/lock"),
            })
            .await
            .map_err(store_err)?
            .token;
        let out = async {
            let mut store = ProjectStore::open_or_create(ctx.as_ref(), project_id).await?;
            let r = f(&mut store)?;
            store.flush().await?;
            Ok(r)
        }
        .await;
        let _ = ctx.persist().release(ReleaseArgs { token }).await;
        out
    }
}

pub struct ProjectStore {
    project_id: String,
    meta: ProjectMeta,
    graph: TaskGraph,
    ctx: Arc<dyn AbilityCtx>,
    dirty_meta: bool,
    dirty_graph: bool,
}

impl ProjectStore {
    async fn open_or_create(ctx: &dyn AbilityCtx, project_id: &str) -> Result<Self, TaskError> {
        let ctx = ctx.retain();
        let index_key = format!("{project_id}/project.yaml");
        let index = load_yaml::<ProjectMeta>(ctx.as_ref(), &index_key).await?;
        let (prefix, meta) = if let Some(meta) = index {
            check_version(&meta.system_version)?;
            let prefix = meta.data_dir.clone();
            validate_prefix(&prefix)?;
            let alt_key = format!("{prefix}/project.yaml");
            if prefix != project_id {
                if let Some(m) = load_yaml::<ProjectMeta>(ctx.as_ref(), &alt_key).await? {
                    check_version(&m.system_version)?;
                    (m.data_dir.clone(), m)
                } else {
                    (prefix, meta)
                }
            } else {
                (prefix, meta)
            }
        } else {
            let meta = ProjectMeta {
                id: project_id.to_string(),
                data_dir: project_id.to_string(),
                default_format: OutputFormat::Json,
                system_version: TASK_SYSTEM_VERSION.to_string(),
            };
            save_yaml(ctx.as_ref(), &index_key, &meta).await?;
            save_json(
                ctx.as_ref(),
                &format!("{project_id}/graph.json"),
                &TaskGraph::default(),
            )
            .await?;
            (project_id.to_string(), meta)
        };

        let graph_key = format!("{prefix}/graph.json");
        let graph = load_json::<TaskGraph>(ctx.as_ref(), &graph_key)
            .await?
            .unwrap_or_default();
        if load_json::<TaskGraph>(ctx.as_ref(), &graph_key)
            .await?
            .is_none()
        {
            save_json(ctx.as_ref(), &graph_key, &graph).await?;
        }

        Ok(Self {
            project_id: project_id.to_string(),
            meta,
            graph,
            ctx,
            dirty_meta: false,
            dirty_graph: false,
        })
    }

    async fn flush(&mut self) -> Result<(), TaskError> {
        if self.dirty_meta {
            self.persist_meta().await?;
        }
        if self.dirty_graph {
            self.persist_graph().await?;
        }
        Ok(())
    }

    async fn persist_meta(&self) -> Result<(), TaskError> {
        let prefix = &self.meta.data_dir;
        save_yaml(
            self.ctx.as_ref(),
            &format!("{prefix}/project.yaml"),
            &self.meta,
        )
        .await?;
        if prefix != &self.project_id {
            save_yaml(
                self.ctx.as_ref(),
                &format!("{}/project.yaml", self.project_id),
                &self.meta,
            )
            .await?;
        }
        Ok(())
    }

    async fn persist_graph(&self) -> Result<(), TaskError> {
        self.graph.validate_dag()?;
        save_json(
            self.ctx.as_ref(),
            &format!("{}/graph.json", self.meta.data_dir),
            &self.graph,
        )
        .await
    }

    pub fn meta(&self) -> &ProjectMeta {
        &self.meta
    }

    pub fn graph(&self) -> &TaskGraph {
        &self.graph
    }

    pub fn resolve_format(&self, override_fmt: Option<&str>) -> Result<OutputFormat, TaskError> {
        match override_fmt {
            Some(s) => OutputFormat::parse(s),
            None => Ok(self.meta.default_format),
        }
    }

    pub fn project_set(
        &mut self,
        data_dir: Option<String>,
        default_format: Option<String>,
    ) -> Result<&ProjectMeta, TaskError> {
        if let Some(fmt) = default_format {
            self.meta.default_format = OutputFormat::parse(&fmt)?;
        }
        if let Some(dir) = data_dir {
            validate_prefix(&dir)?;
            if dir != self.meta.data_dir {
                self.meta.data_dir = dir;
                self.dirty_graph = true;
            }
        }
        self.dirty_meta = true;
        Ok(&self.meta)
    }

    pub fn submit(&mut self, drafts: Vec<TaskDraft>) -> Result<Vec<Task>, TaskError> {
        if drafts.is_empty() {
            return Err(TaskError::Invalid("tasks 不能为空".into()));
        }
        let mut submitted = Vec::new();
        for d in drafts {
            if d.id.is_empty() || d.name.is_empty() {
                return Err(TaskError::Invalid("task id/name 不能为空".into()));
            }
            ensure_no_dup_deps(&d.depends_on)?;
            if self.graph.tasks.contains_key(&d.id) {
                return Err(TaskError::Conflict(format!("任务已存在: {}", d.id)));
            }
            let task = Task {
                id: d.id,
                name: d.name,
                description: d.description.unwrap_or_default(),
                status: TaskStatus::Unclaimed,
                claimed_by: None,
                depends_on: d.depends_on,
                system_version: TASK_SYSTEM_VERSION.to_string(),
            };
            self.graph.tasks.insert(task.id.clone(), task.clone());
            submitted.push(task);
        }
        self.dirty_graph = true;
        Ok(submitted)
    }

    pub fn claim_next(&mut self, claimed_by: &str) -> Result<Option<Task>, TaskError> {
        if claimed_by.is_empty() {
            return Err(TaskError::Invalid("claimed_by 不能为空".into()));
        }
        let Some(id) = self.graph.ready_ids().into_iter().next() else {
            return Ok(None);
        };
        let task = self.graph.tasks.get_mut(&id).unwrap();
        task.status = TaskStatus::Claimed;
        task.claimed_by = Some(claimed_by.to_string());
        let out = task.clone();
        self.dirty_graph = true;
        Ok(Some(out))
    }

    pub fn update(
        &mut self,
        task_id: &str,
        status: TaskStatus,
        claimed_by: Option<&str>,
    ) -> Result<Task, TaskError> {
        let task = self
            .graph
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| TaskError::NotFound(format!("任务不存在: {task_id}")))?;

        if !can_transition(task.status, status) {
            return Err(TaskError::Conflict(format!(
                "非法状态迁移: {} → {}",
                task.status.as_str(),
                status.as_str()
            )));
        }

        if matches!(
            task.status,
            TaskStatus::Claimed | TaskStatus::Running | TaskStatus::Verifying
        ) {
            if let Some(req) = claimed_by {
                match &task.claimed_by {
                    Some(owner) if owner == req => {}
                    Some(owner) => {
                        return Err(TaskError::Conflict(format!(
                            "claimed_by 不匹配: 持有者为 {owner}"
                        )));
                    }
                    None => {
                        return Err(TaskError::Conflict("任务无 claimed_by".into()));
                    }
                }
            }
        }

        if status == TaskStatus::Claimed {
            let who = claimed_by
                .filter(|s| !s.is_empty())
                .ok_or_else(|| TaskError::Invalid("迁移到 claimed 需要 claimed_by".into()))?;
            task.claimed_by = Some(who.to_string());
        }
        if status == TaskStatus::Unclaimed {
            task.claimed_by = None;
        }
        task.status = status;
        let out = task.clone();
        self.dirty_graph = true;
        Ok(out)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct TaskDraft {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

fn validate_project_id(id: &str) -> Result<(), TaskError> {
    if id.is_empty() {
        return Err(TaskError::Invalid("project_id 不能为空".into()));
    }
    validate_prefix(id)
}

fn validate_prefix(id: &str) -> Result<(), TaskError> {
    if id.is_empty() || id.contains("..") || id.starts_with('/') || id.contains('\\') {
        return Err(TaskError::Invalid("data_dir / project_id 非法".into()));
    }
    Ok(())
}

fn check_version(v: &str) -> Result<(), TaskError> {
    if v == TASK_SYSTEM_VERSION {
        Ok(())
    } else {
        Err(TaskError::Version(format!(
            "不支持的任务系统版本: {v}（支持 {TASK_SYSTEM_VERSION}）"
        )))
    }
}

fn store_err(e: ability_support::Error) -> TaskError {
    TaskError::Io(e.to_string())
}

async fn load_yaml<T: serde::de::DeserializeOwned>(
    ctx: &dyn AbilityCtx,
    key: &str,
) -> Result<Option<T>, TaskError> {
    load_codec(ctx, key, "yaml").await
}

async fn load_json<T: serde::de::DeserializeOwned>(
    ctx: &dyn AbilityCtx,
    key: &str,
) -> Result<Option<T>, TaskError> {
    load_codec(ctx, key, "json").await
}

async fn load_codec<T: serde::de::DeserializeOwned>(
    ctx: &dyn AbilityCtx,
    key: &str,
    codec: &str,
) -> Result<Option<T>, TaskError> {
    let out = ctx
        .persist()
        .get(GetArgs {
            owner: Some(OWNER.into()),
            space: None,
            key: key.into(),
            codec: Some(codec.into()),
        })
        .await
        .map_err(store_err)?;
    match out.value {
        Some(v) => Ok(Some(
            serde_json::from_value(v).map_err(|e| TaskError::Io(e.to_string()))?,
        )),
        None => Ok(None),
    }
}

async fn save_yaml<T: serde::Serialize>(
    ctx: &dyn AbilityCtx,
    key: &str,
    value: &T,
) -> Result<(), TaskError> {
    save_codec(ctx, key, "yaml", value).await
}

async fn save_json<T: serde::Serialize>(
    ctx: &dyn AbilityCtx,
    key: &str,
    value: &T,
) -> Result<(), TaskError> {
    save_codec(ctx, key, "json", value).await
}

async fn save_codec<T: serde::Serialize>(
    ctx: &dyn AbilityCtx,
    key: &str,
    codec: &str,
    value: &T,
) -> Result<(), TaskError> {
    let v = serde_json::to_value(value).map_err(|e| TaskError::Io(e.to_string()))?;
    ctx.persist()
        .put(PutArgs {
            owner: Some(OWNER.into()),
            space: None,
            key: key.into(),
            value: v,
            codec: Some(codec.into()),
        })
        .await
        .map_err(store_err)?;
    Ok(())
}

impl Default for TaskHub {
    fn default() -> Self {
        Self::new()
    }
}
