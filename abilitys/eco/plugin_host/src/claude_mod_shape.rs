//! Claude Mod 有界形态：与 `apps/plugin-host/claude-mod.compat.json` 对齐。

use std::path::Path;

use ability_support::Error;
use serde::Deserialize;
use serde_json::Value;

pub const UNSUPPORTED_HOOKS_MODULES: &str = "claude_mod_unsupported:hooks_modules";
pub const SUPPORTED_TIER_HOOKS_COMMAND: &str = "hooks_command";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudeModShape {
    /// command 钩子，或 `modules` 仅指向 `.mjs`/`.js` 单文件（Leaf 测试与轻量 Mod）。
    HooksCommand,
    /// `modules` 含 `.ts`/`.tsx`（Claude 官方 register 入口，无界 UI 运行时）。
    HooksModuleTs,
}

#[derive(Debug, Deserialize)]
struct HooksJson {
    #[serde(default)]
    modules: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct ClaudeCompatFile {
    #[serde(default, rename = "supportedTier")]
    supported_tier: Option<String>,
    #[serde(default, rename = "denyHooksJsonModules")]
    deny_hooks_json_modules: Option<bool>,
    #[serde(default, rename = "unsupportedError")]
    unsupported_error: Option<String>,
}

/// 读取宿主侧 compat 清单（与 Node `readClaudeCompatManifest` 同路径规则）。
pub fn resolve_claude_compat_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("LEAF_PLUGIN_HOST") {
        let script = std::path::PathBuf::from(p);
        if let Some(parent) = script.parent() {
            return parent.join("claude-mod.compat.json");
        }
    }
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../apps/plugin-host/claude-mod.compat.json")
}

pub fn read_claude_compat() -> ClaudeCompatFile {
    let path = resolve_claude_compat_path();
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| "{}".into());
    serde_json::from_str(&raw).unwrap_or(ClaudeCompatFile {
        supported_tier: None,
        deny_hooks_json_modules: None,
        unsupported_error: None,
    })
}

fn hooks_json_modules(plugin_root: &Path) -> Vec<String> {
    let hooks_path = plugin_root.join("hooks").join("hooks.json");
    let raw = match std::fs::read_to_string(&hooks_path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let doc: HooksJson = match serde_json::from_str(&raw) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    doc.modules
        .iter()
        .filter_map(|v| {
            v.as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
        })
        .collect()
}

/// 根据 `hooks/hooks.json` 判定插件形态。
pub fn classify_claude_mod_root(plugin_root: &Path) -> Result<ClaudeModShape, Error> {
    if !plugin_root.join("hooks").join("hooks.json").is_file() {
        return Err(Error::config(format!(
            "不是 Claude Mod 目录（缺少 hooks/hooks.json）: {}",
            plugin_root.display()
        )));
    }
    let modules = hooks_json_modules(plugin_root);
    if modules.iter().any(|m| module_is_typescript_entry(m)) {
        Ok(ClaudeModShape::HooksModuleTs)
    } else {
        Ok(ClaudeModShape::HooksCommand)
    }
}

fn module_is_typescript_entry(rel: &str) -> bool {
    let lower = rel.trim().to_lowercase();
    lower.ends_with(".ts") || lower.ends_with(".tsx")
}

/// 显式配置的插件路径：module tier 在默认契约下不可加载。
pub fn ensure_claude_mod_path_allowed(plugin_root: &Path) -> Result<(), Error> {
    let compat = read_claude_compat();
    if compat.deny_hooks_json_modules != Some(true) {
        return Ok(());
    }
    match classify_claude_mod_root(plugin_root)? {
        ClaudeModShape::HooksCommand => Ok(()),
        ClaudeModShape::HooksModuleTs => {
            let code = compat
                .unsupported_error
                .as_deref()
                .unwrap_or(UNSUPPORTED_HOOKS_MODULES);
            let tier = compat
                .supported_tier
                .as_deref()
                .unwrap_or(SUPPORTED_TIER_HOOKS_COMMAND);
            Err(Error::config(format!(
                "{code}：仅支持 tier {tier}（hooks.json 不得含 modules）；路径 {}",
                plugin_root.display()
            )))
        }
    }
}

/// `vendor_plugins_dir` 展开：module tier 是否应跳过（默认跳过）。
pub fn should_skip_vendor_claude_plugin(plugin_root: &Path) -> Result<bool, Error> {
    let shape = classify_claude_mod_root(plugin_root)?;
    if shape == ClaudeModShape::HooksModuleTs {
        let compat = read_claude_compat();
        if compat.deny_hooks_json_modules == Some(true) {
            if claude_mod_strict_vendor() {
                return Err(Error::config(format!(
                    "{}：vendor 目录含不支持的 module 插件 {}",
                    compat
                        .unsupported_error
                        .as_deref()
                        .unwrap_or(UNSUPPORTED_HOOKS_MODULES),
                    plugin_root.display()
                )));
            }
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn claude_mod_strict_vendor() -> bool {
    std::env::var("LEAF_CLAUDE_MOD_STRICT").as_deref() == Ok("1")
}

pub fn claude_mod_load_strict() -> bool {
    std::env::var("LEAF_CLAUDE_MOD_STRICT").as_deref() != Ok("0")
}
