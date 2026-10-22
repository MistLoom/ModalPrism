//! 装载规格。`source` 须已 install。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{InstanceId, SourceId};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoadSpec {
    pub source: SourceId,
    /// 运行实例名。
    #[serde(rename = "as")]
    pub as_name: InstanceId,
    #[serde(default)]
    pub config: Value,
    /// 覆盖本实例 depends 键 → 已有实例。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bindings: Option<BTreeMap<String, InstanceId>>,
}

impl LoadSpec {
    pub fn new(source: SourceId, as_name: InstanceId) -> Self {
        Self {
            source,
            as_name,
            config: Value::Null,
            bindings: None,
        }
    }

    pub fn with_config(mut self, config: Value) -> Self {
        self.config = config;
        self
    }
}
