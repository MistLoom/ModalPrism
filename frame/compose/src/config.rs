//! 组合根读盘。不是 Ability；Ability `config` 只提供 load/env 方法。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use ability_support::Error;
use serde_json::Value;

#[derive(Default)]
pub struct ConfigCatalog {
    inner: RwLock<HashMap<String, Value>>,
    paths: RwLock<HashMap<String, PathBuf>>,
}

impl ConfigCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load_id(
        &self,
        id: Option<&str>,
        path: impl AsRef<Path>,
    ) -> Result<(String, Value), Error> {
        let path = path.as_ref();
        let value = load_assembly_file(path)?;
        let id = id.map(|s| s.to_string()).unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("default")
                .to_string()
        });
        self.inner
            .write()
            .expect("config catalog")
            .insert(id.clone(), value.clone());
        self.paths
            .write()
            .expect("config catalog")
            .insert(id.clone(), path.to_path_buf());
        Ok((id, value))
    }

    pub fn get(&self, id: &str) -> Option<Value> {
        self.inner.read().expect("config catalog").get(id).cloned()
    }

    pub fn get_str(&self, id: &str, key: &str) -> Option<String> {
        let v = self.get(id)?;
        pointer_str(&v, key)
    }
}

fn load_assembly_file(path: &Path) -> Result<Value, Error> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| Error::io(format!("读取配置失败 {}: {e}", path.display())))?;
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("json") => serde_json::from_str(&raw)
            .map_err(|e| Error::invalid_args(format!("JSON 配置无效: {e}"))),
        _ => serde_yaml::from_str(&raw)
            .map_err(|e| Error::invalid_args(format!("YAML 配置无效: {e}"))),
    }
}

fn pointer_str(value: &Value, key: &str) -> Option<String> {
    let mut cur = value;
    for part in key.split('.') {
        cur = cur.get(part)?;
    }
    match cur {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => Some(cur.to_string()),
    }
}
