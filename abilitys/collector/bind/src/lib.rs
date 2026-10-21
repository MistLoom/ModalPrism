//! 采集进程 Ability 工厂共用绑定期配置。

use std::path::PathBuf;

use ability_support::{Error, Result};
use config::LoadOptions;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BindConfig {
    pub data_dir: PathBuf,
    pub config_dir: PathBuf,
    #[serde(default)]
    pub system: Value,
    #[serde(default)]
    pub proxy_url: Option<String>,
    #[serde(default)]
    pub browser_program: Option<PathBuf>,
}

impl BindConfig {
    pub fn parse(config: &Value) -> Result<Self> {
        serde_json::from_value(config.clone()).map_err(|e| Error::invalid_args(e.to_string()))
    }

    pub fn load_options(&self) -> LoadOptions {
        LoadOptions::from_process()
    }

    pub fn strata_config(&self) -> Value {
        serde_json::json!({
            "dataDir": self.data_dir,
            "system": self.system,
        })
    }
}
