//! 根目录与 key 约束。不把绝对路径交给上层。

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::types::Space;
use ability_support::{Error, Result};

pub const ENV_DATA_ROOT: &str = "LEAF_DATA_ROOT";
pub const ENV_TEMP_ROOT: &str = "LEAF_TEMP_ROOT";

pub fn roots_from_config(config: &Value) -> (PathBuf, PathBuf) {
    let data = config
        .get("data_root")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .or_else(|| std::env::var(ENV_DATA_ROOT).ok().map(PathBuf::from))
        .unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join("data")
        });
    let temp = config
        .get("temp_root")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .or_else(|| std::env::var(ENV_TEMP_ROOT).ok().map(PathBuf::from))
        .unwrap_or_else(|| std::env::temp_dir().join("leaf"));
    (data, temp)
}

pub fn validate_owner(owner: &str) -> Result<()> {
    if owner.is_empty()
        || !owner
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return Err(Error::invalid_args(format!("非法 owner: {owner}")));
    }
    Ok(())
}

pub fn sanitize_key(key: &str) -> Result<PathBuf> {
    if key.is_empty() || key.starts_with('/') || key.starts_with('\\') || key.contains('\\') {
        return Err(Error::invalid_args(format!("非法 key: {key}")));
    }
    let mut out = PathBuf::new();
    for part in key.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains(':') {
            return Err(Error::invalid_args(format!("非法 key: {key}")));
        }
        out.push(part);
    }
    if out.as_os_str().is_empty() {
        return Err(Error::invalid_args(format!("非法 key: {key}")));
    }
    Ok(out)
}

pub fn resolve_path(root: &Path, owner: &str, key: &str) -> Result<PathBuf> {
    validate_owner(owner)?;
    let rel = sanitize_key(key)?;
    Ok(root.join(owner).join(rel))
}

pub fn space_root(space: Space, data: &Path, temp: &Path) -> PathBuf {
    match space {
        Space::Persist => data.to_path_buf(),
        Space::Temp => temp.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_escape() {
        assert!(sanitize_key("../x").is_err());
        assert!(sanitize_key("a/../../b").is_err());
        assert!(sanitize_key("a/b").is_ok());
    }
}
