//! Web 控制台业务逻辑。由 `console` Ability 与 HTTP 适配层调用。

pub mod config_path;
pub mod copy;
pub mod error;
pub mod handlers;

use std::sync::Arc;

use assembly::Services;
use serde_json::Value;

pub use error::{ConsoleError, ConsoleResult};

#[derive(Clone)]
pub struct ConsoleContext {
    pub services: Arc<Services>,
    pub http_addr: String,
}

/// 统一入口，供 Ability `dispatch` 与组合根装配使用。
pub async fn dispatch(ctx: &ConsoleContext, op: &str, args: Value) -> ConsoleResult<Value> {
    handlers::dispatch(ctx, op, args).await
}
