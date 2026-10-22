//! 入口裁剪（框架一等公民）。按实例 id 配置启停与实例配置；协议开放由能力声明目录决定。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Error, Result};
use ability_support::ProtocolKind;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AbilityExpose {
    #[serde(default)]
    pub unknown: UnknownPolicy,
    #[serde(default)]
    pub defaults: CapExpose,
    /// 按实例 id。兼容旧键 `tools` / `capabilities`。
    #[serde(default, alias = "tools", alias = "capabilities")]
    pub abilities: BTreeMap<String, CapExpose>,
}

impl Default for AbilityExpose {
    fn default() -> Self {
        Self::open_all()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownPolicy {
    #[default]
    Defaults,
    Deny,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CapExpose {
    #[serde(default)]
    pub enabled: Option<bool>,
    /// 实例级绑定期配置。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessKind {
    Cli,
    Http,
    Mcp,
    Fun,
    Grpc,
    /// LLM tool_calls 发现与调用边界。
    Tools,
}

impl AccessKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Http => "http",
            Self::Mcp => "mcp",
            Self::Fun => "fun",
            Self::Grpc => "grpc",
            Self::Tools => "tools",
        }
    }

    /// 与 ability-support 协议种类对齐。
    pub fn from_protocol_str(s: &str) -> Option<Self> {
        match s {
            "cli" => Some(Self::Cli),
            "http" => Some(Self::Http),
            "mcp" => Some(Self::Mcp),
            "fun" => Some(Self::Fun),
            "grpc" => Some(Self::Grpc),
            "tools" => Some(Self::Tools),
            _ => None,
        }
    }

    /// 从 Manifest 协议种类映射到运行时门控种类。
    pub fn from_protocol(kind: ProtocolKind) -> Option<Self> {
        match kind {
            ProtocolKind::Cli => Some(Self::Cli),
            ProtocolKind::Http => Some(Self::Http),
            ProtocolKind::Mcp => Some(Self::Mcp),
            ProtocolKind::Fun => Some(Self::Fun),
            ProtocolKind::Grpc => Some(Self::Grpc),
            ProtocolKind::Tools => Some(Self::Tools),
            ProtocolKind::Skill => None,
        }
    }
}

/// 已废弃：CLI 产物由 SDK 独立编译管理。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Runtime,
    ToolBin,
}

pub fn artifact_exe_name(id: &str) -> String {
    id.replace('.', "_")
}

#[derive(Clone, Debug)]
pub struct ResolvedCap {
    pub id: String,
    pub enabled: bool,
    pub config: Value,
}

impl AbilityExpose {
    pub fn open_all() -> Self {
        Self {
            unknown: UnknownPolicy::Defaults,
            defaults: CapExpose {
                enabled: Some(true),
                config: None,
            },
            abilities: BTreeMap::new(),
        }
    }

    pub fn from_value(value: &Value) -> Result<Self> {
        let node = value
            .get("ability_expose")
            .or_else(|| value.get("tool_expose"))
            .or_else(|| value.get("expose"))
            .unwrap_or(value);
        let mut normalized = node.clone();
        if let Some(obj) = normalized.as_object_mut() {
            if !obj.contains_key("abilities") {
                if let Some(caps) = obj.remove("tools").or_else(|| obj.remove("capabilities")) {
                    obj.insert("abilities".into(), caps);
                }
            }
            // 剥离旧字段 access / artifacts
            if let Some(defaults) = obj.get_mut("defaults").and_then(|v| v.as_object_mut()) {
                defaults.remove("access");
                defaults.remove("artifacts");
            }
            if let Some(abilities) = obj.get_mut("abilities").and_then(|v| v.as_object_mut()) {
                for (_k, cap) in abilities.iter_mut() {
                    if let Some(cap_obj) = cap.as_object_mut() {
                        cap_obj.remove("access");
                        cap_obj.remove("artifacts");
                    }
                }
            }
        }
        serde_json::from_value(normalized)
            .map_err(|e| Error::invalid_args(format!("开放配置无效: {e}")))
    }

    pub fn resolve(&self, id: &str) -> ResolvedCap {
        let specific = self.abilities.get(id);
        let listed = specific.is_some();
        let enabled = match (listed, self.unknown) {
            (false, UnknownPolicy::Deny) => false,
            _ => specific
                .and_then(|c| c.enabled)
                .or(self.defaults.enabled)
                .unwrap_or(matches!(self.unknown, UnknownPolicy::Defaults)),
        };
        let config = specific
            .and_then(|c| c.config.clone())
            .or_else(|| self.defaults.config.clone())
            .unwrap_or(Value::Null);
        ResolvedCap {
            id: id.to_string(),
            enabled,
            config,
        }
    }

    pub fn filter_enabled<'a>(&self, registered: impl IntoIterator<Item = &'a str>) -> Vec<String> {
        let mut out: Vec<String> = registered
            .into_iter()
            .filter(|id| self.resolve(id).enabled)
            .map(|s| s.to_string())
            .collect();
        out.sort();
        out
    }

    pub fn is_enabled(&self, id: &str) -> bool {
        self.resolve(id).enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn simplified_expose_loads() {
        let v = json!({
            "ability_expose": {
                "unknown": "deny",
                "defaults": { "enabled": true },
                "abilities": {
                    "read_file": { "enabled": true, "config": { "max": 1 } },
                    "write_file": { "enabled": false }
                }
            }
        });
        let plan = AbilityExpose::from_value(&v).unwrap();
        assert!(plan.is_enabled("read_file"));
        assert!(!plan.is_enabled("write_file"));
        assert_eq!(plan.resolve("read_file").config["max"], 1);
    }
}
