//! 图拓扑任务支撑：模型、落盘、就绪集、认领锁、格式渲染。不实现 Tool。

mod error;
mod format;
mod model;
mod store;

pub use error::TaskError;
pub use format::{graph_summary, project_to_value, task_to_value, tasks_to_value, wrap};
pub use model::{OutputFormat, ProjectMeta, TASK_SYSTEM_VERSION, Task, TaskGraph, TaskStatus};
pub use store::{TaskDraft, TaskHub};
