//! 按生态种类启动 Node 宿主。

use std::path::PathBuf;
use std::sync::Arc;

use ability_support::{AbilityCtx, Result};
use serde::Deserialize;
use serde_json::Value;

use crate::bridge::PluginHostBridge;
use crate::coordinator::SharedHostHandle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginHostKind {
    ClaudeMod,
    Dsh,
}

impl PluginHostKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeMod => "claude_mod",
            Self::Dsh => "dsh",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct PluginEntry {
    pub path: String,
    /// 工具名前缀实例 id（省略则用路径稳定 hash）
    #[serde(default)]
    pub id: Option<String>,
    /// DSH：`thin` | `cordis` | `external` | `auto`（默认，由宿主探测）
    #[serde(default)]
    pub loader: Option<String>,
    /// Cordis 插件配置（与 Harness `cordis.yml` 中该插件的 config 同形）
    #[serde(default)]
    pub config: Option<Value>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct PluginHostConfig {
    #[serde(default)]
    pub plugins: Vec<PluginEntry>,
}

pub struct PluginHostRuntime {
    kind: PluginHostKind,
    shared: SharedHostHandle,
}

impl PluginHostRuntime {
    pub fn new(kind: PluginHostKind) -> Self {
        Self {
            kind,
            shared: SharedHostHandle::new(),
        }
    }

    pub fn hook_names(&self) -> &[String] {
        &[]
    }

    pub async fn hook_names_async(&self) -> Vec<String> {
        self.shared.hook_names().await
    }

    pub fn bridge(&self) -> Option<Arc<PluginHostBridge>> {
        None
    }

    pub async fn bridge_async(&self) -> Option<Arc<PluginHostBridge>> {
        self.shared.bridge().await
    }

    pub fn kind(&self) -> PluginHostKind {
        self.kind
    }

    pub async fn start(&mut self, ctx: &dyn AbilityCtx, config: &PluginHostConfig) -> Result<()> {
        self.shared.register_and_start(ctx, self.kind, config).await
    }
}

pub fn resolve_host_script() -> PathBuf {
    if let Ok(p) = std::env::var("LEAF_PLUGIN_HOST") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../apps/plugin-host/run.mjs")
}

pub fn resolve_plugin_path_public(path: &str) -> String {
    crate::coordinator::resolve_plugin_path(path)
}
