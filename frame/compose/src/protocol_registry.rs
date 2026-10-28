//! 协议中心化注册：聚合 MCP / Tools / Skill / gRPC 声明。

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use ability_core::AbilityHost;
use ability_support::{
    AbilityPackageMeta, CatalogEntry, GrpcInfo, McpInfo, SkillInfo, ToolsInfo, load_ability_package,
};
use frame_core::{AccessKind, Gateway};

use crate::catalog::{WorkspaceCatalog, resolve_protocols};

/// 全局协议注册表（组合根持有）。
#[derive(Clone, Default)]
pub struct ProtocolRegistry {
    packages: BTreeMap<String, AbilityPackageMeta>,
    skills: BTreeMap<String, SkillInfo>,
    grpc: BTreeMap<String, GrpcInfo>,
    mcp: BTreeMap<String, McpInfo>,
    tools: BTreeMap<String, ToolsInfo>,
}

impl ProtocolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn scan_catalog(&mut self, catalog: &WorkspaceCatalog) {
        for entry in catalog.linked_sources() {
            if entry.root.join("ability.yml").is_file() {
                if let Ok(meta) = load_ability_package(&entry.root) {
                    self.ingest(&entry.id, meta);
                }
            }
        }
    }

    pub fn ingest(&mut self, instance_id: &str, meta: AbilityPackageMeta) {
        if let Some(skill) = meta.skill.clone() {
            self.skills.insert(instance_id.to_string(), skill);
        }
        if let Some(grpc) = meta.grpc.clone() {
            self.grpc.insert(instance_id.to_string(), grpc);
        }
        if let Some(mcp) = meta.mcp.clone() {
            self.mcp.insert(instance_id.to_string(), mcp);
        }
        if let Some(tools) = meta.tools.clone() {
            self.tools.insert(instance_id.to_string(), tools);
        }
        self.packages.insert(instance_id.to_string(), meta);
    }

    pub fn package(&self, instance_id: &str) -> Option<&AbilityPackageMeta> {
        self.packages.get(instance_id)
    }

    pub fn supports(
        &self,
        instance_id: &str,
        kind: AccessKind,
        catalog: &WorkspaceCatalog,
    ) -> bool {
        if let Some(pkg) = self.packages.get(instance_id) {
            return pkg
                .protocols
                .iter()
                .any(|p| AccessKind::from_protocol(*p) == Some(kind));
        }
        if let Some(entry) = catalog.get(instance_id) {
            return resolve_protocols(&entry.root).contains(&kind);
        }
        false
    }

    pub fn mcp_tools(&self, catalog: &[CatalogEntry]) -> Vec<McpToolEntry> {
        catalog.iter().map(|e| self.project_mcp_entry(e)).collect()
    }

    pub fn llm_tools(&self, catalog: &[CatalogEntry]) -> Vec<McpToolEntry> {
        catalog.iter().map(|e| self.project_llm_entry(e)).collect()
    }

    fn project_mcp_entry(&self, e: &CatalogEntry) -> McpToolEntry {
        let overrides = self.mcp.get(e.instance.as_str());
        let tool =
            overrides.and_then(|m| m.tools.iter().find(|t| t.method == e.method.to_string()));
        McpToolEntry {
            instance: e.instance.to_string(),
            method: e.method.to_string(),
            name: tool
                .and_then(|t| t.name.clone())
                .unwrap_or_else(|| e.discovery_name(1)),
            description: tool
                .and_then(|t| t.description.clone())
                .unwrap_or_else(|| e.description.clone()),
        }
    }

    fn project_llm_entry(&self, e: &CatalogEntry) -> McpToolEntry {
        let overrides = self.tools.get(e.instance.as_str());
        let tool =
            overrides.and_then(|m| m.tools.iter().find(|t| t.method == e.method.to_string()));
        McpToolEntry {
            instance: e.instance.to_string(),
            method: e.method.to_string(),
            name: tool
                .and_then(|t| t.name.clone())
                .unwrap_or_else(|| e.discovery_name(1)),
            description: tool
                .and_then(|t| t.description.clone())
                .unwrap_or_else(|| e.description.clone()),
        }
    }

    pub fn skill_catalog(&self) -> Vec<&SkillInfo> {
        self.skills.values().collect()
    }

    pub fn grpc_services(&self) -> Vec<&GrpcInfo> {
        self.grpc.values().collect()
    }
}

#[derive(Clone, Debug)]
pub struct McpToolEntry {
    pub instance: String,
    pub method: String,
    pub name: String,
    pub description: String,
}

pub fn sync_protocol_support(host: &AbilityHost, catalog: &WorkspaceCatalog) {
    for entry in catalog.linked_sources() {
        let kinds = resolve_protocols(&entry.root);
        host.set_protocol_support(&entry.id, kinds);
    }
}

pub fn any_supports_gate(
    registry: &ProtocolRegistry,
    catalog: &WorkspaceCatalog,
    gate: AccessKind,
    instance_ids: &[&str],
) -> bool {
    instance_ids
        .iter()
        .any(|id| registry.supports(id, gate, catalog))
}

pub fn build_tool_index(
    gateway: Arc<dyn Gateway>,
    registry: &ProtocolRegistry,
) -> Vec<McpToolEntry> {
    registry.mcp_tools(&gateway.catalog())
}
