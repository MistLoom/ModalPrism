//! 外部插件 Node 宿主桥接，仅供 `claude_mod` / `dsh_plugin` 等 Ability 依赖。

mod bridge;
mod claude_mod_shape;
mod coordinator;
mod doctor;
mod ecosystem_aspect;
mod hooks;
mod host_api;
mod host_surfaces;
mod naming;
mod runtime;
mod user_fanout;
mod wire;

pub use bridge::PluginHostBridge;
pub use claude_mod_shape::{
    ClaudeModShape, SUPPORTED_TIER_HOOKS_COMMAND, UNSUPPORTED_HOOKS_MODULES,
    classify_claude_mod_root, claude_mod_load_strict, ensure_claude_mod_path_allowed,
    should_skip_vendor_claude_plugin,
};
pub use coordinator::{PluginHostCoordinator, coordinator, plugin_instance_id};
pub use doctor::{PluginDoctorReport, diagnose_config, diagnose_config_async, report_to_value};
pub use ecosystem_aspect::EcosystemDispatchAspect;
pub use hooks::{HookKind, classify_hook};
pub use host_api::dispatch_host_api;
pub use host_surfaces::{is_mapped_outbound, mod_outbound_surfaces};
pub use naming::{host_tool_key, llm_prefix, llm_tool_name, parse_llm_tool_name};
pub use runtime::{PluginEntry, PluginHostConfig, PluginHostKind, PluginHostRuntime};
pub use user_fanout::set_user_fanout;
pub use wire::{wire_claude_mod_events, wire_turn_complete};
