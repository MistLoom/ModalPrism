//! 只打开配置与 Strata。不构造调度、出站、付费和访问器。

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use config::{ConfigStore, LoadOptions, Snapshot};
use serde_json::Value;
use strata_client::{Options, Strata};

use crate::error::Error;

pub struct ReadRoot {
    config_dir: PathBuf,
    load_options: LoadOptions,
    config: ConfigStore,
    strata: Strata,
}

impl ReadRoot {
    pub async fn open(data: &Path, config_dir: &Path) -> Result<Self, Error> {
        let load_options = LoadOptions::from_process();
        let snapshot = config::validate(config_dir, &load_options)?;
        refuse_remote(&snapshot, data)?;
        let options = strata_options(&snapshot.system);
        let config = ConfigStore::open(config_dir, load_options.clone())?;
        let strata = Strata::open(data, options).await?;
        Ok(Self {
            config_dir: config_dir.to_path_buf(),
            load_options,
            config,
            strata,
        })
    }

    pub fn config(&self) -> &ConfigStore {
        &self.config
    }

    pub fn strata(&self) -> &Strata {
        &self.strata
    }

    pub(crate) fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub(crate) fn load_options(&self) -> &LoadOptions {
        &self.load_options
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
    usize::try_from(count.saturating_mul(scale)).ok()
}

fn parse_duration(text: &str) -> Option<Duration> {
    let split = text.find(|ch: char| !ch.is_ascii_digit())?;
    let count: u64 = text[..split].parse().ok()?;
    let scale = match &text[split..] {
        "ms" => 1,
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return None,
    };
    Some(Duration::from_millis(count.saturating_mul(scale)))
}

fn refuse_remote(snapshot: &Snapshot, data: &Path) -> Result<(), Error> {
    let system = &snapshot.system;
    if system
        .pointer("/strata/lance/uri")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|uri| !uri.is_empty())
    {
        return Err(Error::new("存储位置未规定"));
    }
    if system
        .pointer("/strata/control_store/backend")
        .and_then(serde_json::Value::as_str)
        == Some("postgres")
    {
        return Err(Error::new("存储位置未规定"));
    }
    if let Some(path) = system
        .pointer("/strata/control_store/path")
        .and_then(serde_json::Value::as_str)
    {
        if normalize(Path::new(path)) != normalize(&data.join("control.db")) {
            return Err(Error::new("存储位置未规定"));
        }
    }
    if let Some(path) = system
        .pointer("/strata/staging/dir")
        .and_then(serde_json::Value::as_str)
    {
        if normalize(Path::new(path)) != normalize(&data.join("staging")) {
            return Err(Error::new("存储位置未规定"));
        }
    }
    Ok(())
}

fn normalize(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}
