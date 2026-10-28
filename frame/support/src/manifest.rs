//! Ability 自身声明。不描述系统角色。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{AbilityId, MethodId};

/// 依赖一项能力（按 id，可选别名与 optional）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dependency {
    pub id: AbilityId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default)]
    pub optional: bool,
}

impl Dependency {
    pub fn required(id: AbilityId) -> Self {
        Self {
            id,
            alias: None,
            optional: false,
        }
    }

    pub fn optional(id: AbilityId) -> Self {
        Self {
            id,
            alias: None,
            optional: true,
        }
    }

    pub fn key(&self) -> String {
        self.alias.clone().unwrap_or_else(|| self.id.to_string())
    }
}

/// 方法声明。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MethodMeta {
    pub id: MethodId,
    #[serde(default)]
    pub description: String,
    pub input_schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
}

impl MethodMeta {
    pub fn new(id: &str, description: impl Into<String>, input_schema: Value) -> Self {
        Self {
            id: MethodId::new(id).expect("method id"),
            description: description.into(),
            input_schema,
            output_schema: None,
        }
    }

    pub fn with_output(mut self, schema: Value) -> Self {
        self.output_schema = Some(schema);
        self
    }
}

/// 自有事件声明。生命周期事件不必列入。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventDecl {
    pub name: String,
    pub dir: EventDir,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub payload_schema: Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventDir {
    Emit,
    Listen,
}

/// 能力声明。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AbilityManifest {
    pub id: AbilityId,
    #[serde(default)]
    pub depends: Vec<Dependency>,
    #[serde(default)]
    pub stateful: bool,
    #[serde(default)]
    pub config_schema: Value,
    pub methods: Vec<MethodMeta>,
    #[serde(default)]
    pub events: Vec<EventDecl>,
}

impl AbilityManifest {
    pub fn method(&self, id: &MethodId) -> Option<&MethodMeta> {
        self.methods.iter().find(|m| m.id == *id)
    }

    pub fn emit_names(&self) -> Vec<&str> {
        self.events
            .iter()
            .filter(|e| e.dir == EventDir::Emit)
            .map(|e| e.name.as_str())
            .collect()
    }

    pub fn listen_decls(&self) -> impl Iterator<Item = &EventDecl> {
        self.events.iter().filter(|e| e.dir == EventDir::Listen)
    }
}
