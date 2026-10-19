//! 声明式查询。`where` 只接受对象，不接受拼接后的字符串。

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::ModelError;
use crate::facet::{FacetKind, FacetSummary};
use crate::id::Timestamp;

/// 查询起点。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryFrom {
    Resources,
    Facets,
    Relations,
}

/// 关系方向。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeDir {
    Out,
    In,
}

/// 血缘方向：来源或派生。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineageDir {
    Up,
    Down,
}

/// 结构化过滤表达式。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Value", into = "Value")]
pub struct Expr(Map<String, Value>);

impl Expr {
    pub fn as_map(&self) -> &Map<String, Value> {
        &self.0
    }
}

impl TryFrom<Value> for Expr {
    type Error = ModelError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::Object(map) if !map.is_empty() => Ok(Self(map)),
            _ => Err(ModelError::ExprNotObject),
        }
    }
}

impl From<Expr> for Value {
    fn from(value: Expr) -> Self {
        Value::Object(value.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FacetSelector {
    pub kind: FacetKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SelectItem {
    Field(String),
    Facet(FacetSelector),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Expand {
    pub rel: String,
    pub dir: EdgeDir,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Search {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector: Option<Vec<f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    pub k: u32,
}

/// 按快照序号、时间或标签读取。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AsOf {
    Snapshot(u64),
    Time(Timestamp),
    Tag(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrderBy {
    pub field: String,
    #[serde(default)]
    pub descending: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Query {
    pub from: QueryFrom,
    #[serde(default, rename = "where", skip_serializing_if = "Option::is_none")]
    pub r#where: Option<Expr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub has_facets: Vec<FacetSelector>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub select: Vec<SelectItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expand: Vec<Expand>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<Search>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of: Option<AsOf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order_by: Vec<OrderBy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

impl Query {
    pub fn validate(&self) -> Result<(), ModelError> {
        if self.limit == Some(0) || self.search.as_ref().is_some_and(|s| s.k == 0) {
            return Err(ModelError::InvalidQuery);
        }
        if self.order_by.iter().any(|item| item.field.is_empty()) {
            return Err(ModelError::InvalidQuery);
        }
        if self.expand.iter().any(|item| item.rel.is_empty()) {
            return Err(ModelError::InvalidQuery);
        }
        Ok(())
    }
}

/// 保留读模型里的摘要类型，避免调用方再定义一套字段。
pub type FacetList = Vec<FacetSummary>;
