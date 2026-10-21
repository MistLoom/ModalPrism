#![allow(dead_code, non_camel_case_types, unused_imports)]
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
    /// 正向代理监听地址（环回），由 egress Ability 在生命周期内绑定。
    #[serde(default)]
    pub proxy_listen: Option<String>,
    /// Push 入站监听地址（环回），由 push_ingress Ability 绑定。
    #[serde(default)]
    pub push_listen: Option<String>,
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
