//! 配置路径白名单与符号链接校验。

use std::path::{Component, Path, PathBuf};

use crate::error::{ConsoleError, ConsoleResult};
use crate::ConsoleContext;

const ALLOWED_ROOTS: &[&str] = &["sources", "presets"];

/// 相对路径（使用 `/`），必须在白名单内。
pub fn normalize_rel_path(path: &str) -> ConsoleResult<PathBuf> {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed.contains('\\') {
        return Err(ConsoleError::bad_request("路径无效"));
    }
    let rel = Path::new(trimmed);
    for component in rel.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            _ => return Err(ConsoleError::bad_request("路径无效")),
        }
    }
    let first = rel
        .components()
        .next()
        .and_then(|c| c.as_os_str().to_str())
        .unwrap_or("");
    if !ALLOWED_ROOTS.contains(&first) {
        return Err(ConsoleError::bad_request("路径不在允许范围内"));
    }
    if !rel.extension().is_some_and(|e| e == "yaml" || e == "yml") {
        return Err(ConsoleError::bad_request("仅允许 YAML 文件"));
    }
    Ok(rel.to_path_buf())
}

/// 解析为磁盘路径，并确认落在配置目录内（含符号链接解析）。
pub async fn resolve_config_file(state: &ConsoleContext, rel: &Path) -> ConsoleResult<PathBuf> {
    let (_, config_dir) = crate::context::layout(state).await?;
    let joined = config_dir.join(rel);
    let canonical_root = config_dir
        .canonicalize()
        .map_err(|e| ConsoleError::internal(e.to_string()))?;
    let canonical_file = if joined.exists() {
        joined
            .canonicalize()
            .map_err(|e| ConsoleError::internal(e.to_string()))?
    } else {
        let parent = joined
            .parent()
            .ok_or_else(|| ConsoleError::bad_request("路径无效"))?;
        if parent.exists() {
            parent
                .canonicalize()
                .map_err(|e| ConsoleError::internal(e.to_string()))?
                .join(
                    joined
                        .file_name()
                        .ok_or_else(|| ConsoleError::bad_request("路径无效"))?,
                )
        } else {
            return Err(ConsoleError::not_found("文件不存在"));
        }
    };
    if !canonical_file.starts_with(&canonical_root) {
        return Err(ConsoleError::bad_request("路径不在允许范围内"));
    }
    Ok(canonical_file)
}

pub async fn list_whitelist_files(state: &ConsoleContext) -> ConsoleResult<Vec<String>> {
    let (_, root) = crate::context::layout(state).await?;
    let mut paths = Vec::new();
    for dir in ALLOWED_ROOTS {
        collect_yaml(&root, Path::new(dir), &mut paths)?;
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn collect_yaml(config_root: &Path, rel_dir: &Path, out: &mut Vec<String>) -> ConsoleResult<()> {
    let dir = config_root.join(rel_dir);
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(&dir).map_err(|e| ConsoleError::internal(e.to_string()))? {
        let entry = entry.map_err(|e| ConsoleError::internal(e.to_string()))?;
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let sub = rel_dir.join(name);
            collect_yaml(config_root, &sub, out)?;
        } else if path.extension().is_some_and(|e| e == "yaml" || e == "yml") {
            let rel = path
                .strip_prefix(config_root)
                .map_err(|_| ConsoleError::internal("路径解析失败"))?;
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}
