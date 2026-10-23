//! 不可变配置快照。

use std::collections::BTreeMap;

use serde_json::Value;

use crate::node::Origin;

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub generation: u64,
    pub system: Value,
    pub modules: BTreeMap<String, Value>,
    pub origins: BTreeMap<(String, String), Origin>,
}
