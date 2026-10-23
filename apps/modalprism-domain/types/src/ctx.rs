//! 只读数据入口的公共契约。
//!
//! `types` 的 JSON 投影只有一份实现，入参是这里定义的窄接口：组合根持有的
//! `ReadRoot` 与采集进程的 `Services` 各自实现它，因此两个进程对同一数据目录
//! 给出逐字段相同的输出。

use std::path::Path;

use config::{ConfigStore, LoadOptions};
use strata_client::Strata;

use crate::root::ReadRoot;

/// 只读投影需要的最小上下文。
pub trait ReadCtx: Send + Sync {
    /// 已加载配置快照。
    fn config(&self) -> &ConfigStore;

    /// 数据读写门面。
    fn strata(&self) -> &Strata;

    /// 配置目录，供 `config_inspect` 重新校验。
    fn config_dir(&self) -> &Path;

    /// 配置加载选项，与 `config_dir` 一起决定校验结果。
    fn load_options(&self) -> &LoadOptions;
}

impl ReadCtx for ReadRoot {
    fn config(&self) -> &ConfigStore {
        &self.config
    }

    fn strata(&self) -> &Strata {
        &self.strata
    }

    fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    fn load_options(&self) -> &LoadOptions {
        &self.load_options
    }
}
