//! Web 控制台业务逻辑（无 HTTP）。由 `console-api` 接入层调用。

pub mod config_path;
pub mod copy;
pub mod error;
pub mod handlers;
pub mod invoke;

use std::sync::Arc;

use collector_boot::BootResources;
use frame_core::Gateway;

pub use error::{ConsoleError, ConsoleResult};

#[derive(Clone)]
pub struct ConsoleContext {
    pub gateway: Arc<dyn Gateway>,
    pub services: Arc<BootResources>,
    pub http_addr: String,
}
