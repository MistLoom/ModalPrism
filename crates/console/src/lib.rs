//! Web 控制台业务逻辑（无 HTTP）。由 `console-api` 接入层调用。

pub mod config_path;
pub mod copy;
pub mod error;
pub mod handlers;

use std::sync::Arc;

use host::HostResources;

pub use error::{ConsoleError, ConsoleResult};

#[derive(Clone)]
pub struct ConsoleContext {
    pub services: Arc<HostResources>,
    pub http_addr: String,
}
