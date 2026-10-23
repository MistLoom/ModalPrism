//! 只打开配置与 Strata。不构造调度、出站、付费和访问器。

use std::path::{Path, PathBuf};

use config::{ConfigStore, LoadOptions};
use strata_client::{Options, Strata, check_storage};

use crate::error::Error;

pub struct ReadRoot {
    pub(crate) config_dir: PathBuf,
    pub(crate) load_options: LoadOptions,
    pub(crate) config: ConfigStore,
    pub(crate) strata: Strata,
}

impl ReadRoot {
    pub async fn open(data: &Path, config_dir: &Path) -> Result<Self, Error> {
        let load_options = LoadOptions::from_process();
        let snapshot = config::validate(config_dir, &load_options)?;
        check_storage(&snapshot.system, data).map_err(Error::new)?;
        let options = Options::from_system(&snapshot.system)?;
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
}
