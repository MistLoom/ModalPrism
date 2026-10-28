//! `ability.yml` 声明。

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::protocol::ProtocolKind;
use super::version::validate_framework_version;
use crate::manifest::{Dependency, EventDecl, EventDir};

/// 方法声明。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MethodDecl {
    pub id: String,
    #[serde(default)]
    pub description: String,
    pub input: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
}

/// 依赖声明（YAML 简写）。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DepDecl {
    Simple(String),
    Full {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        alias: Option<String>,
        #[serde(default)]
        optional: bool,
    },
}

impl DepDecl {
    pub fn into_dependency(self) -> Result<Dependency, String> {
        match self {
            Self::Simple(id) => {
                let aid = crate::AbilityId::new(&id).map_err(|e| e.to_string())?;
                Ok(Dependency::required(aid))
            }
            Self::Full {
                id,
                alias,
                optional,
            } => {
                let aid = crate::AbilityId::new(&id).map_err(|e| e.to_string())?;
                let mut dep = if optional {
                    Dependency::optional(aid)
                } else {
                    Dependency::required(aid)
                };
                dep.alias = alias;
                Ok(dep)
            }
        }
    }
}

/// 域内出站 topic 声明（不经 EventBus，如 kb hook 投递）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutboundTopicDeclYaml {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub payload_schema: Value,
}

/// 事件声明（YAML）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventDeclYaml {
    pub name: String,
    pub dir: crate::manifest::EventDir,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub payload_schema: Value,
}

impl EventDeclYaml {
    pub fn into_event_decl(self) -> EventDecl {
        EventDecl {
            name: self.name,
            dir: self.dir,
            description: self.description,
            payload_schema: self.payload_schema,
        }
    }
}

/// 能力实现运行时（与线协议传输无关）。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    #[default]
    Rust,
    Node,
    Python,
    Cangjie,
}

/// 能力本体声明（`ability.yml`）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AbilityDecl {
    /// 声明 schema 版本，须与 [`FRAMEWORK_VERSION`] 一致。
    pub framework_version: String,
    pub id: String,
    /// 其他应用可否调用。默认仅属主应用可调用。
    #[serde(default)]
    pub open: bool,
    #[serde(default)]
    pub depends: Vec<DepDecl>,
    #[serde(default)]
    pub stateful: bool,
    #[serde(default)]
    pub config_schema: Value,
    pub methods: Vec<MethodDecl>,
    #[serde(default)]
    pub events: Vec<EventDeclYaml>,
    /// 域内可订阅出站 topic（不经 `ctx.emit` / EventBus）。
    #[serde(default)]
    pub outbound_topics: Vec<OutboundTopicDeclYaml>,
    /// 使用自定义 `Ability` 实现（非 handler trait）。
    #[serde(default)]
    pub factory: bool,
    /// 本能力支持的协议清单（须与协议目录声明一致）。
    pub protocols: Vec<ProtocolKind>,
    /// 内置单标记：能力由框架随仓维护（随框架更新面整包走）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub leaf_builtin: bool,
}

impl AbilityDecl {
    pub fn validate(&self) -> Result<(), String> {
        validate_framework_version("ability.yml", Some(&self.framework_version))?;
        crate::AbilityId::new(&self.id).map_err(|e| e.to_string())?;
        if self.methods.is_empty() {
            return Err("ability.yml 至少声明一个 method".into());
        }
        if self.protocols.is_empty() {
            return Err("ability.yml 须声明 protocols（至少一项）".into());
        }
        let mut protocol_names = HashSet::new();
        for p in &self.protocols {
            if !protocol_names.insert(*p) {
                return Err(format!("ability.yml protocols 重复: {}", p.as_str()));
            }
        }
        for m in &self.methods {
            crate::MethodId::new(&m.id).map_err(|e| e.to_string())?;
            super::protocol::validate_schema_fragment(&m.input)?;
            if m.input.get("type").and_then(|v| v.as_str()) != Some("object") {
                return Err(format!(
                    "ability.yml method {} 的 input 须含 type: object",
                    m.id
                ));
            }
            if let Some(out) = &m.output {
                super::protocol::validate_schema_fragment(out)?;
                if out.get("type").and_then(|v| v.as_str()) != Some("object") {
                    return Err(format!(
                        "ability.yml method {} 的 output 须含 type: object",
                        m.id
                    ));
                }
            }
        }
        let mut event_names = HashSet::new();
        for e in &self.events {
            if !event_names.insert(&e.name) {
                return Err(format!("ability.yml 事件名重复: {}", e.name));
            }
            if e.dir == EventDir::Emit && e.payload_schema.is_null() {
                return Err(format!(
                    "ability.yml emit 事件 {} 须提供 payload_schema",
                    e.name
                ));
            }
            super::protocol::validate_schema_fragment(&e.payload_schema)?;
        }
        let mut topic_names = HashSet::new();
        for t in &self.outbound_topics {
            if !topic_names.insert(&t.name) {
                return Err(format!("ability.yml outbound_topics 名重复: {}", t.name));
            }
            if t.payload_schema.is_null() {
                return Err(format!(
                    "ability.yml outbound_topics {} 须提供 payload_schema",
                    t.name
                ));
            }
            super::protocol::validate_schema_fragment(&t.payload_schema)?;
        }
        Ok(())
    }

    pub fn uses_handlers(&self) -> bool {
        !self.factory
    }

    /// 方法 id → `src/logic/<file>.rs` 模块名。
    pub fn logic_module_name(method_id: &str) -> String {
        method_id.replace('.', "_").replace('-', "_")
    }
}
