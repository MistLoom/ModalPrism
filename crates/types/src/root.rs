//! 只打开配置与 Strata。不构造调度、出站、付费和访问器。

use std::path::{Component, Path, PathBuf};

use config::{ConfigStore, LoadOptions, Snapshot};
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
        let config = ConfigStore::open(config_dir, load_options.clone())?;
        let strata = Strata::open(data, Options::default()).await?;
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
