//! Planner、Runner、节奏与健康。任务租约在控制存储的队列上。

mod cadence;
mod error;
mod health;
mod persist;
mod planner;
mod queue;
mod runner;

pub use cadence::{CadenceInput, next_interval};
pub use error::PlannerError;
pub use health::{
    Health, Thresholds, canonical_source_id, counts_toward_health, health_accessor_subject,
    health_source_subject,
};
pub use persist::{MemoryPersist, Persist};
pub use planner::{Capacities, Planner, SourcePlan, SourceStatus};
pub use queue::{MemoryQueue, Queue};
pub use runner::{Classified, Handler, Quality, ReceiptBody, Receipts, TaskSuccess, backoff};
