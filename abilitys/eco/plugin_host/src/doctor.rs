//! 插件路径与 Node / 宿主脚本前提检查。

use std::path::Path;

use serde::Serialize;
use serde_json::{Value, json};

use crate::claude_mod_shape::ensure_claude_mod_path_allowed;
use crate::coordinator::plugin_entry_to_init;
use crate::runtime::{
    PluginHostConfig, PluginHostKind, resolve_host_script, resolve_plugin_path_public,
};

#[derive(Debug, Serialize)]
pub struct PluginDoctorReport {
    pub ok: bool,
    pub node: bool,
    pub host_script: String,
    pub host_script_exists: bool,
    pub plugins: Vec<PluginPathStatus>,
    /// 内嵌 DSH Cordis 栈与桥版本（仅当清单含 DSH 插件时填充）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dsh_embedded: Option<Value>,
    /// Node `probe` 加载结果（需运行时 AbilityCtx）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probe: Option<Value>,
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct PluginPathStatus {
    pub path: String,
    pub exists: bool,
}

pub fn diagnose_config(cfg: &PluginHostConfig) -> PluginDoctorReport {
    let node = node_available();
    let host_script = resolve_host_script();
    let host_script_exists = host_script.is_file();
    let mut errors = Vec::new();
    if !node {
        errors.push("未找到 Node，请安装或设置 LEAF_PLUGIN_NODE".into());
    }
    if !host_script_exists {
        errors.push(format!(
            "未找到插件宿主脚本 {}，可设置 LEAF_PLUGIN_HOST",
            host_script.display()
        ));
    }
    let plugins = cfg
        .plugins
        .iter()
        .map(|e| {
            let abs = resolve_plugin_path_public(&e.path);
            let exists = plugin_path_ok(&abs);
            if !exists {
                errors.push(format!("插件路径不存在或无效: {abs}"));
            }
            PluginPathStatus { path: abs, exists }
        })
        .collect();
    if cfg.plugins.is_empty() {
        errors.push("plugins 列表为空".into());
    }
    let dsh_embedded = diagnose_dsh_embedded(&host_script, cfg, &mut errors);
    let ok = errors.is_empty();
    PluginDoctorReport {
        ok,
        node,
        host_script: host_script.display().to_string(),
        host_script_exists,
        plugins,
        dsh_embedded,
        probe: None,
        errors,
    }
}

/// 静态诊断 + 可选 Node 加载探测。
pub async fn diagnose_config_async(
    ctx: &dyn ability_support::AbilityCtx,
    cfg: &PluginHostConfig,
    kind: PluginHostKind,
) -> PluginDoctorReport {
    let mut report = diagnose_config(cfg);
    if kind == PluginHostKind::ClaudeMod {
        for entry in &cfg.plugins {
            let abs = resolve_plugin_path_public(&entry.path);
            if let Err(e) = ensure_claude_mod_path_allowed(std::path::Path::new(&abs)) {
                report.ok = false;
                report.errors.push(e.to_string());
            }
        }
    }
    if !report.ok || !report.node || !report.host_script_exists || cfg.plugins.is_empty() {
        return report;
    }
    let plugins = cfg
        .plugins
        .iter()
        .map(|e| plugin_entry_to_init(kind, e))
        .collect::<Vec<_>>();
    let script = resolve_host_script();
    match crate::bridge::PluginHostBridge::probe(script, plugins, ctx.retain()).await {
        Ok(frame) => {
            if let Some(entries) = frame.get("entries").and_then(|v| v.as_array()) {
                for entry in entries {
                    if entry.get("ok").and_then(|v| v.as_bool()) == Some(false) {
                        report.ok = false;
                        for err in entry
                            .get("errors")
                            .and_then(|v| v.as_array())
                            .into_iter()
                            .flatten()
                        {
                            if let Some(s) = err.as_str() {
                                report.errors.push(s.to_string());
                            }
                        }
                    }
                }
            }
            report.probe = Some(frame);
        }
        Err(e) => {
            report.ok = false;
            report.errors.push(e.to_string());
        }
    }
    report
}

fn diagnose_dsh_embedded(
    host_script: &Path,
    cfg: &PluginHostConfig,
    errors: &mut Vec<String>,
) -> Option<Value> {
    let needs_cordis = cfg.plugins.iter().any(|e| {
        e.loader.as_deref() == Some("cordis")
            || e.loader.as_deref() == Some("external")
            || e.loader.as_deref() == Some("auto")
            || e.loader.is_none()
    });
    if !needs_cordis {
        return None;
    }
    let host_root = host_script.parent().unwrap_or(Path::new("."));
    let compat_path = host_root.join("dsh-runtime.compat.json");
    if !compat_path.is_file() {
        errors.push(format!("缺少 DSH 兼容清单 {}", compat_path.display()));
        return Some(json!({ "ok": false, "reason": "missing_compat_manifest" }));
    }
    let compat: Value = match std::fs::read_to_string(&compat_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
    {
        Some(v) => v,
        None => {
            errors.push("dsh-runtime.compat.json 无法解析".into());
            return Some(json!({ "ok": false }));
        }
    };
    let strict = std::env::var("LEAF_DSH_STRICT").as_deref() == Ok("1");
    let packages = compat.get("packages").and_then(|v| v.as_object());
    let mut pkg_report = json!({});
    let mut stack_ok = true;
    if let Some(pkgs) = packages {
        for (name, spec) in pkgs {
            let installed = read_pkg_version(host_root, name);
            let pinned = spec.get("pinned").and_then(|v| v.as_str());
            let optional = spec.get("optional").and_then(|v| v.as_bool()) == Some(true);
            let ok = version_matches(installed.as_deref(), spec);
            pkg_report[name] = json!({
                "installed": installed,
                "pinned": pinned,
                "ok": ok,
            });
            if !ok && !(optional && installed.is_none()) {
                let msg = format!(
                    "{name}: 已安装 {:?}，与兼容清单不一致（pinned {:?}）",
                    installed, pinned
                );
                if strict && !optional {
                    errors.push(msg);
                    stack_ok = false;
                }
            }
        }
    }
    Some(json!({
        "ok": stack_ok,
        "bridgeVersion": compat.get("bridgeVersion"),
        "strict": strict,
        "loaders": compat.get("loaders"),
        "packages": pkg_report,
    }))
}

fn read_pkg_version(host_root: &Path, name: &str) -> Option<String> {
    let path = host_root
        .join("node_modules")
        .join(name)
        .join("package.json");
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<Value>(&raw)
        .ok()?
        .get("version")?
        .as_str()
        .map(String::from)
}

fn version_matches(installed: Option<&str>, spec: &Value) -> bool {
    let installed = match installed {
        Some(v) => v,
        None => return spec.get("optional").and_then(|v| v.as_bool()) == Some(true),
    };
    if spec.get("pinned").and_then(|v| v.as_str()) == Some(installed) {
        return true;
    }
    if let Some(majors) = spec.get("supportedMajor").and_then(|v| v.as_array()) {
        let major = installed
            .split('.')
            .next()
            .and_then(|s| s.parse::<u64>().ok());
        if let Some(m) = major {
            if majors.iter().any(|x| x.as_u64() == Some(m)) {
                return true;
            }
        }
    }
    if let Some(prefixes) = spec.get("supportedPrefixes").and_then(|v| v.as_array()) {
        if prefixes
            .iter()
            .filter_map(|p| p.as_str())
            .any(|pfx| installed.starts_with(pfx))
        {
            return true;
        }
    }
    false
}

pub fn report_to_value(r: &PluginDoctorReport) -> Value {
    serde_json::to_value(r).unwrap_or(json!({ "ok": r.ok }))
}

fn node_available() -> bool {
    std::process::Command::new(std::env::var("LEAF_PLUGIN_NODE").unwrap_or_else(|_| "node".into()))
        .arg("--version")
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn plugin_path_ok(abs: &str) -> bool {
    let joined = Path::new(abs);
    let p = joined
        .canonicalize()
        .unwrap_or_else(|_| joined.to_path_buf());
    if p.is_file() && p.extension().is_some_and(|e| e == "mjs") {
        return true;
    }
    if p.join("hooks/hooks.json").is_file() {
        return true;
    }
    dsh_npm_package_entry_ok(&p)
}

/// 与 `apps/plugin-host/src/dsh/loader.mjs` 的 `resolveImportUrl` 对齐。
fn dsh_npm_package_entry_ok(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    let pkg_path = dir.join("package.json");
    let raw = match std::fs::read_to_string(&pkg_path) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let pkg: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let rel = match npm_package_entry_rel(&pkg) {
        Some(r) => r,
        None => return false,
    };
    let entry = dir.join(rel);
    entry.is_file()
}

fn npm_package_entry_rel(pkg: &Value) -> Option<String> {
    if let Some(dot) = pkg.get("exports").and_then(|e| e.get(".")) {
        if let Some(s) = dot.as_str() {
            return Some(s.to_string());
        }
        if let Some(obj) = dot.as_object() {
            for key in ["default", "import", "node"] {
                if let Some(s) = obj.get(key).and_then(|v| v.as_str()) {
                    return Some(s.to_string());
                }
            }
        }
    }
    if let Some(dsh) = pkg.get("dsh") {
        if let Some(s) = dsh
            .get("bundle")
            .and_then(|b| b.get("main"))
            .and_then(|v| v.as_str())
        {
            return Some(s.to_string());
        }
        if let Some(s) = dsh.get("main").and_then(|v| v.as_str()) {
            return Some(s.to_string());
        }
    }
    for key in ["module", "main"] {
        if let Some(s) = pkg.get(key).and_then(|v| v.as_str()) {
            return Some(s.to_string());
        }
    }
    Some("index.js".into())
}
