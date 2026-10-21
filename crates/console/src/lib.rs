//! Web 控制台业务逻辑（无 HTTP）。由 `console-api` 接入层调用。

pub mod config_path;
pub mod context;
pub mod copy;
pub mod error;
pub mod handlers;
pub mod invoke;
pub mod strata_port;

use std::sync::Arc;

use frame_core::Gateway;

pub use error::{ConsoleError, ConsoleResult};

#[derive(Clone)]
pub struct ConsoleContext {
    pub gateway: Arc<dyn Gateway>,
    pub http_addr: String,
}
