use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::SessionError;

/// 规范化槽位 / kind / 分组键。
pub(crate) fn normalize_ident(raw: impl AsRef<str>, what: &str) -> Result<String, SessionError> {
    let s = raw.as_ref().trim();
    if s.is_empty() {
        return Err(SessionError::invalid(format!("{what}为空")));
    }
    if s.len() > 64 {
        return Err(SessionError::invalid(format!("{what}过长: {s}")));
    }
    if s.chars().any(|c| c.is_control() || c == ',') {
        return Err(SessionError::invalid(format!("{what}含非法字符: {s}")));
    }
    Ok(s.to_string())
}

/// 外部资源引用。`kind` / `id` 由调用方定义，会话层不解释。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtRef {
    pub kind: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub meta: Map<String, Value>,
}

impl ExtRef {
    pub fn new(kind: impl Into<String>, id: impl Into<String>) -> Result<Self, SessionError> {
        Ok(Self {
            kind: normalize_ident(kind.into(), "引用 kind")?,
            id: {
                let id = id.into();
                let t = id.trim();
                if t.is_empty() {
                    return Err(SessionError::invalid("引用 id 为空"));
                }
                if t.len() > 256 {
                    return Err(SessionError::invalid("引用 id 过长"));
                }
                t.to_string()
            },
            meta: Map::new(),
        })
    }

    pub fn with_meta(mut self, meta: Map<String, Value>) -> Self {
        self.meta = meta;
        self
    }
}

/// 动态引用表：槽位名由调用方定义。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefMap(BTreeMap<String, ExtRef>);

impl RefMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bind(&mut self, slot: impl Into<String>, refer: ExtRef) -> Result<(), SessionError> {
        let slot = normalize_ident(slot.into(), "引用槽位")?;
        self.0.insert(slot, refer);
        Ok(())
    }

    pub fn unbind(&mut self, slot: &str) -> Option<ExtRef> {
        self.0.remove(slot)
    }

    pub fn get(&self, slot: &str) -> Option<&ExtRef> {
        self.0.get(slot)
    }

    pub fn has_kind(&self, kind: &str) -> bool {
        self.0.values().any(|r| r.kind == kind)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &ExtRef)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub(crate) fn apply(&mut self, op: &RefOp) -> Result<bool, SessionError> {
        match op {
            RefOp::Bind { slot, refer } => {
                self.bind(slot, refer.clone())?;
                Ok(true)
            }
            RefOp::Unbind { slot } => Ok(self.unbind(slot).is_some()),
            RefOp::Set(map) => {
                if map == self {
                    Ok(false)
                } else {
                    *self = map.clone();
                    Ok(true)
                }
            }
        }
    }
}

/// 引用变更。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefOp {
    Bind { slot: String, refer: ExtRef },
    Unbind { slot: String },
    Set(RefMap),
}

/// 调用方定义的度量（键由调用方命名，本层不解释单位）。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MetricMap(BTreeMap<String, f64>);

impl MetricMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, key: impl Into<String>, value: f64) -> Result<(), SessionError> {
        let key = normalize_ident(key.into(), "度量键")?;
        self.0.insert(key, value);
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<f64> {
        self.0.get(key).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, f64)> {
        self.0.iter().map(|(k, v)| (k.as_str(), *v))
    }

    pub(crate) fn merge(&mut self, other: &MetricMap) {
        for (k, v) in &other.0 {
            self.0.insert(k.clone(), *v);
        }
    }
}

/// 扩展袋合并（同键覆盖）。
pub(crate) fn merge_ext(dst: &mut Map<String, Value>, src: &Map<String, Value>) {
    for (k, v) in src {
        dst.insert(k.clone(), v.clone());
    }
}
