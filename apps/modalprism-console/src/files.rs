//! 配置文件的读写面。
//!
//! 控制台允许浏览与编辑采集进程的配置目录（与 `--config` 指向同一份），
//! 但只开放 YAML/JSON 声明文件，路径永远限制在目录内部。

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use axum::Json;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiError;

/// 只允许声明文件。日志、数据库、暂存文件一概不可触碰。
const EXTENSIONS: &[&str] = &["yaml", "yml", "json"];

/// 单文件上限 2 MiB：配置文件远小于此，超出多半是拿错了目录。
const MAX_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Deserialize)]
pub struct FileQuery {
    pub path: String,
}

#[derive(Deserialize)]
pub struct FileBody {
    pub path: String,
    pub content: String,
}

/// 列出配置目录内的全部声明文件（递归）。
pub fn list(config_dir: &Path) -> Result<Json<Value>, ApiError> {
    if !config_dir.is_dir() {
        return Err(
            ApiError::missing("配置目录").with_hint("确认控制台 --config 指向采集进程的配置目录")
        );
    }
    let mut files = Vec::new();
    walk(config_dir, "", &mut files);
    files.sort_by(|a, b| {
        let a = a.get("path").and_then(Value::as_str).unwrap_or_default();
        let b = b.get("path").and_then(Value::as_str).unwrap_or_default();
        a.cmp(b)
    });
    Ok(Json(json!({ "files": files })))
}

/// 读取一份声明文件的内容。
pub fn read(config_dir: &Path, raw: &str) -> Result<Json<Value>, ApiError> {
    let target = resolve(config_dir, raw)?;
    let meta = fs::metadata(&target).map_err(|_| ApiError::missing("该配置文件"))?;
    if meta.len() > MAX_BYTES {
        return Err(ApiError::invalid("文件超过 2 MiB，不在界面中打开"));
    }
    let content = fs::read_to_string(&target)
        .map_err(|error| ApiError::invalid(format!("读取失败: {error}")))?;
    Ok(Json(json!({
        "path": clean(raw),
        "content": content,
        "size": meta.len(),
        "modifiedMs": modified_ms(&meta),
    })))
}

/// 写入声明文件；父目录不存在时一并创建。
pub fn write(config_dir: &Path, body: &FileBody) -> Result<Json<Value>, ApiError> {
    if body.content.len() as u64 > MAX_BYTES {
        return Err(ApiError::invalid("内容超过 2 MiB 上限"));
    }
    let target = resolve(config_dir, &body.path)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| ApiError::invalid(format!("目录创建失败: {error}")))?;
    }
    fs::write(&target, &body.content)
        .map_err(|error| ApiError::invalid(format!("写入失败: {error}")))?;
    tracing::info!(path = %body.path, "配置文件已写入");
    Ok(Json(json!({ "ok": true })))
}

/// 删除声明文件。
pub fn delete(config_dir: &Path, raw: &str) -> Result<Json<Value>, ApiError> {
    let target = resolve(config_dir, raw)?;
    fs::remove_file(&target).map_err(|_| ApiError::missing("该配置文件"))?;
    tracing::info!(path = %raw, "配置文件已删除");
    Ok(Json(json!({ "ok": true })))
}

/// 把界面传来的相对路径解析为配置目录内的绝对路径。
fn resolve(config_dir: &Path, raw: &str) -> Result<PathBuf, ApiError> {
    let trimmed = raw.trim().trim_start_matches(['/', '\\']);
    if trimmed.is_empty() {
        return Err(ApiError::invalid("缺少文件路径"));
    }
    let relative = Path::new(trimmed);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ApiError::invalid("路径必须位于配置目录内"));
    }
    let name = relative
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if !allowed_name(name) {
        return Err(ApiError::invalid("只允许 YAML/JSON 声明文件"));
    }
    let target = config_dir.join(relative);
    // 已存在的文件做一次规范路径比对，防符号链接逃逸；新建文件由 join 保证约束。
    if let Ok(canonical) = target.canonicalize()
        && let Ok(root) = config_dir.canonicalize()
        && !canonical.starts_with(&root)
    {
        return Err(ApiError::invalid("路径越出了配置目录"));
    }
    Ok(target)
}

fn allowed_name(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

fn clean(raw: &str) -> String {
    raw.trim().trim_start_matches(['/', '\\']).to_string()
}

fn modified_ms(meta: &fs::Metadata) -> Option<u64> {
    meta.modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as u64)
}

fn walk(dir: &Path, prefix: &str, files: &mut Vec<Value>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().to_string();
        let rel = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if file_type.is_dir() {
            walk(&entry.path(), &rel, files);
        } else if file_type.is_file() && allowed_name(&name) {
            let meta = entry.metadata().ok();
            files.push(json!({
                "path": rel,
                "size": meta.as_ref().map(fs::Metadata::len).unwrap_or(0),
                "modifiedMs": meta.as_ref().and_then(modified_ms),
            }));
        }
    }
}
