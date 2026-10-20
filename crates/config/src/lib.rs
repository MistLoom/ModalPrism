//! 配置快照。
//!
//! 一次加载把 YAML 目录合并成不可变快照。业务通过 [`validate`]、[`diff`]、[`explain`]
//! 和 [`ConfigStore`] 使用它，不读取未校验的文件。

mod check;
mod diff;
mod error;
mod explain;
mod interpolate;
mod load;
mod merge;
mod node;
mod presets;
mod schema;
mod scope;
mod snapshot;
mod store;
mod yaml;

pub use diff::{Change, diff};
pub use error::{Diagnostic, Diagnostics, Error};
pub use explain::{FieldOrigin, explain};
pub use load::{EnvSource, LoadOptions, validate};
pub use presets::{BuiltinPreset, builtin_presets};
pub use scope::overlay_system_policy;
pub use snapshot::Snapshot;
pub use store::{ConfigStore, DEFAULT_DEBOUNCE, RestartChange, WatchHandle, watch, watch_hangup};
