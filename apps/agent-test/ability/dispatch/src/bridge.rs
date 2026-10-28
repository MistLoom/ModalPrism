use std::collections::HashMap;

use ability_support::CatalogEntry;

use ai_protocol::ToolDef;
use serde_json::json;

pub fn discovery_to_function_name(name: &str) -> String {
    name.replace('.', "_")
}

fn method_counts(catalog: &[CatalogEntry]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for e in catalog {
        *counts.entry(e.instance.to_string()).or_insert(0) += 1;
    }
    counts
}

pub fn discovery_name(entry: &CatalogEntry, counts: &HashMap<String, usize>) -> String {
    entry.discovery_name(*counts.get(entry.instance.as_str()).unwrap_or(&1))
}

/// catalog 生成 LLM 侧工具定义；排除 dispatch 自身
pub fn catalog_to_tool_defs(catalog: &[CatalogEntry]) -> Vec<ToolDef> {
    let counts = method_counts(catalog);
    catalog
        .iter()
        .filter(|e| e.ability_id.as_str() != "dispatch")
        .map(|e| {
            let name = discovery_to_function_name(&discovery_name(e, &counts));
            let parameters = if e.input_schema.is_null() {
                json!({ "type": "object", "properties": {} })
            } else {
                e.input_schema.clone()
            };
            ToolDef::function(name, e.description.clone(), parameters)
        })
        .collect()
}
