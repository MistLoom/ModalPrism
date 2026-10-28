use std::path::Path;

use ability_support::Error;
use serde_json::{Map, Value, json};

use crate::format::ConfigFormat;

/// 从文件加载配置
pub fn load_file(path: &Path, format: ConfigFormat) -> Result<Value, Error> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| Error::io(format!("读取配置失败 {}: {e}", path.display())))?;
    let format = match format {
        ConfigFormat::Auto => ConfigFormat::detect(path),
        other => other,
    };
    load_str(&raw, format)
}

/// 从字符串解析配置
pub fn load_str(raw: &str, format: ConfigFormat) -> Result<Value, Error> {
    match format {
        ConfigFormat::Auto => Err(Error::invalid_args("load_str 需要显format，不能使auto")),
        ConfigFormat::Json => serde_json::from_str(raw)
            .map_err(|e| Error::invalid_args(format!("JSON 配置无效: {e}"))),
        ConfigFormat::Yaml => {
            let v: Value = serde_yaml::from_str(raw)
                .map_err(|e| Error::invalid_args(format!("YAML 配置无效: {e}")))?;
            Ok(v)
        }
        ConfigFormat::Ini | ConfigFormat::Conf => parse_ini_like(raw),
        ConfigFormat::Env => parse_env(raw),
    }
}

fn parse_ini_like(raw: &str) -> Result<Value, Error> {
    let conf = ini::Ini::load_from_str(raw)
        .map_err(|e| Error::invalid_args(format!("INI/CONF 配置无效: {e}")))?;
    let mut root = Map::new();
    for (section, prop) in conf.iter() {
        let mut map = Map::new();
        for (k, v) in prop.iter() {
            map.insert(k.to_string(), Value::String(v.to_string()));
        }
        let key = section.unwrap_or("default").to_string();
        root.insert(key, Value::Object(map));
    }
    Ok(Value::Object(root))
}

fn parse_env(raw: &str) -> Result<Value, Error> {
    let mut map = Map::new();
    for (idx, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            return Err(Error::invalid_args(format!("ENV {} 行缺'='", idx + 1)));
        };
        let k = k.trim();
        let v = unquote(v.trim());
        map.insert(k.to_string(), Value::String(v));
    }
    // 合并进程环境（不覆盖文件键）
    for (k, v) in std::env::vars() {
        map.entry(k).or_insert(Value::String(v));
    }
    Ok(Value::Object(map))
}

fn unquote(s: &str) -> String {
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// 仅从进程环境收集（可选前缀）
pub fn load_process_env(prefix: Option<&str>) -> Value {
    let mut map = Map::new();
    for (k, v) in std::env::vars() {
        if let Some(p) = prefix {
            if let Some(rest) = k.strip_prefix(p) {
                map.insert(rest.to_string(), json!(v));
            }
        } else {
            map.insert(k, json!(v));
        }
    }
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_yaml_and_ini() {
        let y = load_str("kb:\n  root: /tmp/kb\n", ConfigFormat::Yaml).unwrap();
        assert_eq!(y["kb"]["root"], "/tmp/kb");
        let i = load_str("[kb]\nroot=/data\n", ConfigFormat::Ini).unwrap();
        assert_eq!(i["kb"]["root"], "/data");
    }
}
