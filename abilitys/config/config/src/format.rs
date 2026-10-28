use std::path::{Path, PathBuf};
use std::str::FromStr;

use ability_support::Error;
use serde::{Deserialize, Serialize};

/// 配置格式
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConfigFormat {
    #[default]
    Auto,
    Json,
    Yaml,
    Ini,
    Conf,
    Env,
}

impl FromStr for ConfigFormat {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "json" => Ok(Self::Json),
            "yaml" | "yml" => Ok(Self::Yaml),
            "ini" => Ok(Self::Ini),
            "conf" => Ok(Self::Conf),
            "env" => Ok(Self::Env),
            other => Err(Error::invalid_args(format!("未知配置格式: {other}"))),
        }
    }
}

impl ConfigFormat {
    pub fn detect(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref()
        {
            Some("json") => Self::Json,
            Some("yaml") | Some("yml") => Self::Yaml,
            Some("ini") => Self::Ini,
            Some("conf") | Some("cfg") => Self::Conf,
            Some("env") => Self::Env,
            _ => Self::Json,
        }
    }
}

/// 解析路径：支`id:` 前缀占位（仅路径形式；id 目录Catalog 处理）
pub fn resolve_path(spec: &str) -> Result<PathBuf, Error> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err(Error::invalid_args("配置路径为空"));
    }
    Ok(PathBuf::from(spec))
}
