//! 全局单一 Node 宿主：合并 claude_mod / dsh_plugin 插件列表，共享桥接。

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::{Arc, OnceLock};

use ability_support::{AbilityCtx, Error, Result};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::bridge::PluginHostBridge;
use crate::ecosystem_aspect::EcosystemDispatchAspect;
use crate::runtime::{PluginEntry, PluginHostConfig, PluginHostKind, resolve_host_script};

static COORDINATOR: OnceLock<Arc<PluginHostCoordinator>> = OnceLock::new();

pub fn coordinator() -> Arc<PluginHostCoordinator> {
    COORDINATOR
        .get_or_init(|| Arc::new(PluginHostCoordinator::new()))
        .clone()
}

pub struct PluginHostCoordinator {
    inner: Mutex<CoordinatorInner>,
}

struct CoordinatorInner {
    bridge: Option<Arc<PluginHostBridge>>,
    hook_names: Vec<String>,
    /// 已合并的 init 插件 JSON（去重按 path+kind）
    init_plugins: Vec<Value>,
    seen_keys: HashSet<String>,
    aspect_registered: bool,
    tool_check_enabled: bool,
}

impl PluginHostCoordinator {
    fn new() -> Self {
        Self {
            inner: Mutex::new(CoordinatorInner {
                bridge: None,
                hook_names: Vec::new(),
                init_plugins: Vec::new(),
                seen_keys: HashSet::new(),
                aspect_registered: false,
                tool_check_enabled: false,
            }),
        }
    }

    /// 注册插件并确保宿主已启动（新增插件时会重启 Node 以全量加载）。
    pub async fn register_and_start(
        &self,
        ctx: &dyn AbilityCtx,
        kind: PluginHostKind,
        config: &PluginHostConfig,
    ) -> Result<()> {
        if config.plugins.is_empty() {
            return Ok(());
        }
        let mut g = self.inner.lock().await;
        let mut added = false;
        for entry in &config.plugins {
            let key = plugin_dedupe_key(kind, entry);
            if g.seen_keys.insert(key) {
                g.init_plugins.push(plugin_entry_to_init(kind, entry));
                added = true;
            }
        }
        if kind == PluginHostKind::ClaudeMod {
            // 有 CC 插件即可能在加载后注册 tool.check
            g.tool_check_enabled = true;
        }
        let plugins = g.init_plugins.clone();
        let script = resolve_host_script();
        if !script.is_file() {
            return Err(Error::config(format!(
                "未找到插件宿主脚本 {}，可设置 LEAF_PLUGIN_HOST",
                script.display()
            )));
        }
        g.bridge = None;
        let (bridge, ready) = PluginHostBridge::spawn(script, plugins, ctx.retain()).await?;
        g.hook_names = ready
            .get("hooks")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        g.tool_check_enabled = g.hook_names.iter().any(|n| n == "tool.check");
        g.bridge = Some(bridge);
        if !g.aspect_registered {
            register_aspect_once(ctx, &mut g)?;
        }
        Ok(())
    }

    pub async fn bridge(&self) -> Option<Arc<PluginHostBridge>> {
        self.inner.lock().await.bridge.clone()
    }

    pub async fn hook_names(&self) -> Vec<String> {
        self.inner.lock().await.hook_names.clone()
    }

    pub async fn tool_check_enabled(&self) -> bool {
        self.inner.lock().await.tool_check_enabled
    }

    /// 新 Gateway 引导前清空（测试与热重载）。
    pub async fn reset(&self) {
        let mut g = self.inner.lock().await;
        *g = CoordinatorInner {
            bridge: None,
            hook_names: Vec::new(),
            init_plugins: Vec::new(),
            seen_keys: HashSet::new(),
            aspect_registered: false,
            tool_check_enabled: false,
        };
    }
}

fn register_aspect_once(ctx: &dyn AbilityCtx, g: &mut CoordinatorInner) -> Result<()> {
    if g.aspect_registered {
        return Ok(());
    }
    let coord = coordinator();
    ctx.register_aspect(Arc::new(EcosystemDispatchAspect::new(coord)))?;
    g.aspect_registered = true;
    Ok(())
}

fn plugin_dedupe_key(kind: PluginHostKind, entry: &PluginEntry) -> String {
    format!("{}:{}", kind.as_str(), resolve_plugin_path(&entry.path))
}

pub fn plugin_instance_id(entry: &PluginEntry) -> String {
    if let Some(id) = &entry.id {
        if !id.is_empty() {
            return sanitize_id(id);
        }
    }
    let path = resolve_plugin_path(&entry.path);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    format!("p{:08x}", h.finish() as u32)
}

fn sanitize_id(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn resolve_plugin_path(path: &str) -> String {
    let p = Path::new(path);
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .ok()
            .map(|cwd| cwd.join(p))
            .unwrap_or_else(|| p.to_path_buf())
    };
    joined
        .canonicalize()
        .map(|c| c.to_string_lossy().into_owned())
        .unwrap_or_else(|_| joined.to_string_lossy().into_owned())
}

pub fn plugin_entry_to_init(kind: PluginHostKind, entry: &PluginEntry) -> Value {
    let instance_id = plugin_instance_id(entry);
    let mut obj = serde_json::Map::new();
    obj.insert("kind".into(), json!(kind.as_str()));
    obj.insert("path".into(), json!(resolve_plugin_path(&entry.path)));
    obj.insert("instanceId".into(), json!(instance_id));
    obj.insert("origin".into(), json!(kind.as_str()));
    if let Some(loader) = &entry.loader {
        obj.insert("loader".into(), json!(loader));
    }
    if let Some(cfg) = &entry.config {
        obj.insert("config".into(), cfg.clone());
    }
    Value::Object(obj)
}

/// 供旧 `PluginHostRuntime` 转发的共享句柄。
pub struct SharedHostHandle {
    coord: Arc<PluginHostCoordinator>,
}

impl SharedHostHandle {
    pub fn new() -> Self {
        Self {
            coord: coordinator(),
        }
    }

    pub async fn register_and_start(
        &self,
        ctx: &dyn AbilityCtx,
        kind: PluginHostKind,
        config: &PluginHostConfig,
    ) -> Result<()> {
        self.coord.register_and_start(ctx, kind, config).await
    }

    pub async fn bridge(&self) -> Option<Arc<PluginHostBridge>> {
        self.coord.bridge().await
    }

    pub async fn hook_names(&self) -> Vec<String> {
        self.coord.hook_names().await
    }

    pub fn kind_hint(&self, kind: PluginHostKind) -> PluginHostKind {
        kind
    }
}
