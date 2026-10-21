//! Strata Ability 共用：从绑定期配置打开 `Strata` 与参数解析。

use std::path::PathBuf;
use std::sync::Arc;

use ability_support::{Error, Result};
use serde::Deserialize;
use serde_json::Value;
use strata_client::{Options, Strata};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenConfig {
    pub data_dir: PathBuf,
    #[serde(default)]
    pub system: Value,
}

pub async fn open_strata(config: &Value) -> Result<Arc<Strata>> {
    let cfg: OpenConfig = serde_json::from_value(config.clone())
        .map_err(|e| Error::invalid_args(e.to_string()))?;
    let options = strata_options(&cfg.system);
    let strata = Strata::open(&cfg.data_dir, options)
        .await
        .map_err(|e| Error::failed(e.to_string()))?;
    Ok(Arc::new(strata))
}

pub fn invalid_args(message: impl Into<String>) -> Error {
    Error::invalid_args(message.into())
}

pub fn failed(message: impl Into<String>) -> Error {
    Error::failed(message.into())
}

pub fn parse<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T> {
    serde_json::from_value(value.clone()).map_err(|_| invalid_args("spec 无效"))
}

pub fn field<'a>(args: &'a Value, key: &str) -> Result<&'a Value> {
    args.get(key).ok_or_else(|| invalid_args("spec 无效"))
}

pub fn text_field(args: &Value, key: &str) -> Result<String> {
    field(args, key)?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| invalid_args("spec 无效"))
}

pub fn normalize_args(args: Value) -> Result<Value> {
    match args {
        Value::Null => Ok(Value::Object(Default::default())),
        Value::Object(_) => Ok(args),
        _ => Err(invalid_args("spec 无效")),
    }
}

fn strata_options(system: &Value) -> Options {
    let mut options = Options::default();
    if let Some(size) = system
        .pointer("/strata/blob/inline_size_threshold")
        .and_then(Value::as_str)
        .and_then(parse_size)
    {
        options.blob_inline_threshold = size;
    }
    if let Some(size) = system
        .pointer("/strata/blob/dedicated_size_threshold")
        .and_then(Value::as_str)
        .and_then(parse_size)
    {
        options.blob_dedicated_threshold = size;
    }
    if let Some(ttl) = system
        .pointer("/strata/staging/ttl")
        .and_then(Value::as_str)
        .and_then(parse_duration)
    {
        options.staging_ttl = ttl;
    }
    options
}

fn parse_size(text: &str) -> Option<usize> {
    let split = text.find(|ch: char| !ch.is_ascii_digit())?;
    let count: u64 = text[..split].parse().ok()?;
    let scale = match &text[split..] {
        "B" => 1,
        "KiB" => 1024,
        "MiB" => 1024 * 1024,
        "GiB" => 1024 * 1024 * 1024,
        _ => return None,
    };
    usize::try_from(count * scale).ok()
}

fn parse_duration(text: &str) -> Option<std::time::Duration> {
    let split = text.find(|ch: char| !ch.is_ascii_digit())?;
    let count: u64 = text[..split].parse().ok()?;
    let unit = &text[split..];
    let secs = match unit {
        "s" => count,
        "m" => count * 60,
        "h" => count * 3600,
        _ => return None,
    };
    Some(std::time::Duration::from_secs(secs))
}
