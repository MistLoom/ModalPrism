//! 带文件位置的配置树。

use std::collections::BTreeMap;

use serde_json::{Number, Value};

/// 一个字段的来源。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    pub file: String,
    pub line: u32,
    pub preset_id: Option<String>,
    pub via_preset: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub file: String,
    pub line: u32,
    pub preset_id: Option<String>,
    pub data: Data,
}

#[derive(Clone, Debug)]
pub enum Data {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Array(Vec<Node>),
    Map(Vec<(String, Node)>),
}

impl Node {
    pub fn map(file: impl Into<String>, line: u32) -> Self {
        Self {
            file: file.into(),
            line,
            preset_id: None,
            data: Data::Map(Vec::new()),
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self.data, Data::Null)
    }

    pub fn as_str(&self) -> Option<&str> {
        match &self.data {
            Data::String(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_map(&self) -> Option<&[(String, Node)]> {
        match &self.data {
            Data::Map(entries) => Some(entries),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&Node> {
        self.as_map()?
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, node)| node)
    }

    pub fn dig(&self, path: &str) -> Option<&Node> {
        let mut current = self;
        if path.is_empty() {
            return Some(current);
        }
        for part in path.split('.') {
            current = current.get(part)?;
        }
        Some(current)
    }

    pub fn stamp_preset(&mut self, id: &str) {
        self.preset_id = Some(id.to_string());
        match &mut self.data {
            Data::Array(items) => {
                for item in items {
                    item.stamp_preset(id);
                }
            }
            Data::Map(entries) => {
                for (_, child) in entries {
                    child.stamp_preset(id);
                }
            }
            _ => {}
        }
    }

    pub fn lower(&self, selected: Option<&str>) -> (Value, BTreeMap<String, Origin>) {
        let mut origins = BTreeMap::new();
        let value = lower_at(self, "", selected, &mut origins);
        (value, origins)
    }
}

fn lower_at(
    node: &Node,
    path: &str,
    selected: Option<&str>,
    origins: &mut BTreeMap<String, Origin>,
) -> Value {
    match &node.data {
        Data::Map(entries) => {
            let mut map = serde_json::Map::new();
            for (key, child) in entries {
                let child_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                map.insert(key.clone(), lower_at(child, &child_path, selected, origins));
            }
            if entries.is_empty() && !path.is_empty() {
                origins.insert(path.to_string(), origin_of(node, selected));
            }
            Value::Object(map)
        }
        Data::Array(items) => {
            if !path.is_empty() {
                origins.insert(path.to_string(), origin_of(node, selected));
            }
            Value::Array(
                items
                    .iter()
                    .map(|item| lower_at(item, path, selected, origins))
                    .collect(),
            )
        }
        other => {
            if !path.is_empty() {
                origins.insert(path.to_string(), origin_of(node, selected));
            }
            scalar(other)
        }
    }
}

fn origin_of(node: &Node, selected: Option<&str>) -> Origin {
    let via_preset = match (&node.preset_id, selected) {
        (Some(writer), Some(selected)) if writer != selected => Some(selected.to_string()),
        _ => None,
    };
    Origin {
        file: node.file.clone(),
        line: node.line,
        preset_id: node.preset_id.clone(),
        via_preset,
    }
}

fn scalar(data: &Data) -> Value {
    match data {
        Data::Null => Value::Null,
        Data::Bool(value) => Value::Bool(*value),
        Data::Int(value) => Value::Number((*value).into()),
        Data::Float(value) => Number::from_f64(*value)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        Data::String(value) => Value::String(value.clone()),
        Data::Array(_) | Data::Map(_) => Value::Null,
    }
}

/// 策略与模块树中的叶子路径。数组算一个叶子。
pub fn leaves<'a>(node: &'a Node, prefix: &str, out: &mut Vec<Leaf<'a>>) {
    match &node.data {
        Data::Map(entries) => {
            if entries.is_empty() && !prefix.is_empty() {
                out.push(Leaf {
                    path: prefix.to_string(),
                    line: node.line,
                    file: &node.file,
                });
                return;
            }
            for (key, child) in entries {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                leaves(child, &path, out);
            }
        }
        _ => {
            if !prefix.is_empty() {
                out.push(Leaf {
                    path: prefix.to_string(),
                    line: node.line,
                    file: &node.file,
                });
            }
        }
    }
}

pub struct Leaf<'a> {
    pub path: String,
    pub line: u32,
    pub file: &'a str,
}
