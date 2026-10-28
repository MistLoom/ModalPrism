//! 方法发现摘要。供端口列出调用面，不是 Tool 身份。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{AbilityId, DiscoveryPayload, Error, InstanceId, Lifecycle, MethodId, Result};

/// 已装实例上的一个可调用方法。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub instance: InstanceId,
    pub ability_id: AbilityId,
    pub method: MethodId,
    pub description: String,
    pub stateful: bool,
    pub phase: Lifecycle,
    pub input_schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
}

/// 把实例摘要展开为端口 / dispatch 用的方法面。
pub fn method_catalog(instances: &[DiscoveryPayload]) -> Vec<CatalogEntry> {
    let mut out = Vec::new();
    for d in instances {
        for method in &d.methods {
            out.push(CatalogEntry {
                instance: d.instance.clone(),
                ability_id: d.ability_id.clone(),
                method: method.id.clone(),
                description: method.description.clone(),
                stateful: d.stateful,
                phase: d.phase,
                input_schema: method.input_schema.clone(),
                output_schema: method.output_schema.clone(),
            });
        }
    }
    out.sort_by(|a, b| {
        a.instance
            .as_str()
            .cmp(b.instance.as_str())
            .then(a.method.as_str().cmp(b.method.as_str()))
    });
    out
}

/// 按谓词过滤方法面。
pub fn filter_method_catalog<F>(catalog: &[CatalogEntry], pred: F) -> Vec<CatalogEntry>
where
    F: Fn(&CatalogEntry) -> bool,
{
    catalog.iter().filter(|e| pred(e)).cloned().collect()
}

/// 统计各实例的方法数（供发现名生成）。
pub fn instance_method_counts(catalog: &[CatalogEntry]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for e in catalog {
        *counts.entry(e.instance.to_string()).or_insert(0) += 1;
    }
    counts
}

impl CatalogEntry {
    /// 发现名：单方法实例用 instance，否则 `instance.method`。
    pub fn discovery_name(&self, method_count: usize) -> String {
        if method_count <= 1 {
            self.instance.to_string()
        } else {
            format!("{}.{}", self.instance, self.method)
        }
    }
}

/// 把端口发现名解析为 instance + method。
pub fn resolve_target(raw: &str, catalog: &[CatalogEntry]) -> Result<(InstanceId, MethodId)> {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for e in catalog {
        *counts.entry(e.instance.as_str()).or_insert(0) += 1;
    }

    let same: Vec<&CatalogEntry> = catalog
        .iter()
        .filter(|e| e.instance.as_str() == raw)
        .collect();
    if same.len() == 1 {
        return Ok((same[0].instance.clone(), same[0].method.clone()));
    }
    if same.len() > 1 {
        return Err(Error::invalid_args(format!(
            "实例 {raw} 有多个方法，请指定 method"
        )));
    }

    for e in catalog {
        let n = e.discovery_name(*counts.get(e.instance.as_str()).unwrap_or(&1));
        if n == raw {
            return Ok((e.instance.clone(), e.method.clone()));
        }
    }

    // 方法名本身可含点（如 `echo.say`），从最右分割点向左尝试全部分割。
    let bytes = raw.as_bytes();
    let mut search = bytes.len();
    while let Some(pos) = raw[..search].rfind('.') {
        let (inst, meth) = (&raw[..pos], &raw[pos + 1..]);
        if let Some(e) = catalog
            .iter()
            .find(|e| e.instance.as_str() == inst && e.method.as_str() == meth)
        {
            return Ok((e.instance.clone(), e.method.clone()));
        }
        search = pos;
    }

    Err(Error::not_found(raw))
}
