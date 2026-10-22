//! 能力 CLI 独立编译。

use std::path::Path;
use std::process::Command;

use ability_support::CliInfo;

use crate::catalog::{WorkspaceCatalog, workspace_root};

/// 编译声明了 `cli/info.yml` 的能力二进制。
pub fn build_clis(filter: Option<&str>, profile: &str) -> Result<(), String> {
    let ws = workspace_root();
    let catalog = WorkspaceCatalog::discover();
    let mut ok = 0usize;
    let mut fail = 0usize;
    for entry in catalog.linked_sources() {
        if let Some(want) = filter {
            if entry.id != want {
                continue;
            }
        }
        let root = &entry.root;
        let info_path = root.join("cli").join("info.yml");
        if !info_path.is_file() {
            continue;
        }
        let pkg = package_name_from_root(root)?;
        let cli_info: CliInfo = ability_support::load_cli_info(&info_path)?;
        let bin = cli_info.binary.clone();
        tracing::info!(package = %pkg, bin = %bin, "building cli");
        let status = Command::new("cargo")
            .current_dir(&ws)
            .args(["build", "-p", &pkg, "--bin", &bin, "--profile", profile])
            .status()
            .map_err(|e| e.to_string())?;
        if status.success() {
            ok += 1;
            let out = ws.join("target").join(profile).join(exe_name(&bin));
            tracing::info!(path = %out.display(), "built");
        } else {
            fail += 1;
        }
    }
    tracing::info!(ok, fail, "cli build finished");
    if fail > 0 {
        return Err(format!("CLI 编译失败 {fail} 个"));
    }
    Ok(())
}

fn package_name_from_root(root: &Path) -> Result<String, String> {
    let toml = std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| e.to_string())?;
    for line in toml.lines() {
        if let Some(rest) = line.strip_prefix("name = ") {
            return Ok(rest.trim().trim_matches('"').to_string());
        }
    }
    Err("Cargo.toml 缺少 name".into())
}

fn exe_name(bin: &str) -> String {
    #[cfg(windows)]
    {
        format!("{bin}.exe")
    }
    #[cfg(not(windows))]
    {
        bin.to_string()
    }
}
