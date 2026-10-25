//! 配置快照。
//!
//! 一次加载把 YAML 目录合并成不可变快照。业务通过 [`validate`]、[`diff`]、[`explain`]
//! 和 [`ConfigStore`] 使用它，不读取未校验的文件。

// 策略体的 JSON Schema 是一个深嵌套的 `json!` 字面量，默认递归上限放不下。
#![recursion_limit = "512"]

mod author;
mod check;
mod diff;
mod duration;
mod error;
mod explain;
mod inspect;
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

pub use author::{
    Draft, Entry, Saved, check_slug, create, entries, list, proxy, remove, templates, to_yaml,
    toggle, update,
};
pub use diff::{Change, diff};
pub use duration::parse as duration;
pub use error::{Diagnostic, Diagnostics, Error};
pub use explain::{FieldOrigin, explain};
pub use inspect::{domain_of_module, domain_of_path, modules};
pub use load::{EnvSource, LoadOptions, validate};
pub use presets::{BuiltinPreset, builtin_presets};
pub use scope::overlay_system_policy;
pub use snapshot::Snapshot;
pub use store::{ConfigStore, DEFAULT_DEBOUNCE, RestartChange, WatchHandle, watch, watch_hangup};
pub use yaml::parse_value as parse_yaml;
