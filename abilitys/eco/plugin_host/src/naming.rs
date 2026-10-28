//! 模型工具名：`eco__{origin}__{instanceId}__{toolName}`

use super::runtime::PluginHostKind;

const ECO_PREFIX: &str = "eco__";

pub fn llm_prefix(kind: PluginHostKind) -> &'static str {
    match kind {
        PluginHostKind::ClaudeMod => "eco__claude_mod__",
        PluginHostKind::Dsh => "eco__dsh__",
    }
}

pub fn llm_tool_name(origin: &str, instance_id: &str, tool_name: &str) -> String {
    format!("eco__{origin}__{instance_id}__{tool_name}")
}

pub fn host_tool_key(instance_id: &str, tool_name: &str) -> String {
    format!("{instance_id}::{tool_name}")
}

/// 解析 LLM 名 → Node 宿主内部 key（`instance::tool`）。
pub fn parse_llm_tool_name(llm_name: &str) -> Option<String> {
    if !llm_name.starts_with(ECO_PREFIX) {
        return None;
    }
    let rest = llm_name.strip_prefix(ECO_PREFIX)?;
    let parts: Vec<&str> = rest.splitn(4, "__").collect();
    if parts.len() < 2 {
        return None;
    }
    let origin = parts[0];
    if origin != "dsh" && origin != "claude_mod" {
        return None;
    }
    if parts.len() >= 3 {
        let instance_id = parts[1];
        let tool_name = parts[2..].join("__");
        return Some(host_tool_key(instance_id, &tool_name));
    }
    // 兼容旧名 `eco__dsh__tool`（无 instance）
    let tool_name = parts[1..].join("__");
    Some(host_tool_key("default", &tool_name))
}

pub fn origin_from_frame(origin: &str) -> Option<PluginHostKind> {
    match origin {
        "claude_mod" => Some(PluginHostKind::ClaudeMod),
        "dsh" => Some(PluginHostKind::Dsh),
        _ => None,
    }
}
