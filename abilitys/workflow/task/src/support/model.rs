//! 任务与工程数据模型。

use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use super::error::TaskError;

/// 实现支持的任务系统协议版本。
pub const TASK_SYSTEM_VERSION: &str = "1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Unclaimed,
    Claimed,
    Running,
    Verifying,
    Completed,
    Failed,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unclaimed => "unclaimed",
            Self::Claimed => "claimed",
            Self::Running => "running",
            Self::Verifying => "verifying",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub status: TaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_by: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub system_version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub id: String,
    /// 相对 root 或绝对路径；决定工程数据目录。
    pub data_dir: String,
    #[serde(default = "default_format")]
    pub default_format: OutputFormat,
    pub system_version: String,
}

fn default_format() -> OutputFormat {
    OutputFormat::Json
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    #[default]
    Json,
    Md,
}

impl OutputFormat {
    pub fn parse(s: &str) -> Result<Self, TaskError> {
        match s {
            "json" => Ok(Self::Json),
            "md" => Ok(Self::Md),
            other => Err(TaskError::Invalid(format!("非法 format: {other}"))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Md => "md",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TaskGraph {
    #[serde(default)]
    pub tasks: HashMap<String, Task>,
}

impl TaskGraph {
    /// 校验依赖存在且无环。
    pub fn validate_dag(&self) -> Result<(), TaskError> {
        for (id, t) in &self.tasks {
            for dep in &t.depends_on {
                if !self.tasks.contains_key(dep) {
                    return Err(TaskError::Invalid(format!("任务 {id} 依赖不存在: {dep}")));
                }
            }
        }
        let mut indeg: HashMap<String, usize> = self
            .tasks
            .values()
            .map(|t| (t.id.clone(), t.depends_on.len()))
            .collect();
        let mut q: VecDeque<String> = indeg
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(k, _)| k.clone())
            .collect();
        let mut seen = 0usize;
        while let Some(n) = q.pop_front() {
            seen += 1;
            for t in self.tasks.values() {
                if t.depends_on.iter().any(|d| d == &n) {
                    if let Some(c) = indeg.get_mut(&t.id) {
                        *c = c.saturating_sub(1);
                        if *c == 0 {
                            q.push_back(t.id.clone());
                        }
                    }
                }
            }
        }
        if seen != self.tasks.len() {
            return Err(TaskError::Invalid("任务依赖存在环".into()));
        }
        Ok(())
    }

    /// 就绪：unclaimed 且前置均 completed。按 id 字典序。
    pub fn ready_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .tasks
            .values()
            .filter(|t| {
                t.status == TaskStatus::Unclaimed
                    && t.depends_on.iter().all(|d| {
                        self.tasks
                            .get(d)
                            .map(|p| p.status == TaskStatus::Completed)
                            .unwrap_or(false)
                    })
            })
            .map(|t| t.id.clone())
            .collect();
        ids.sort();
        ids
    }

    pub fn claimable_tasks(&self) -> Vec<&Task> {
        self.ready_ids()
            .into_iter()
            .filter_map(|id| self.tasks.get(&id))
            .collect()
    }
}

/// 合法单步状态迁移。
pub fn can_transition(from: TaskStatus, to: TaskStatus) -> bool {
    use TaskStatus::*;
    matches!(
        (from, to),
        (Unclaimed, Claimed)
            | (Claimed, Running)
            | (Claimed, Failed)
            | (Running, Verifying)
            | (Running, Failed)
            | (Verifying, Completed)
            | (Verifying, Failed)
    )
}

pub fn ensure_no_dup_deps(deps: &[String]) -> Result<(), TaskError> {
    let mut s = HashSet::new();
    for d in deps {
        if !s.insert(d.as_str()) {
            return Err(TaskError::Invalid(format!("重复依赖: {d}")));
        }
    }
    Ok(())
}
