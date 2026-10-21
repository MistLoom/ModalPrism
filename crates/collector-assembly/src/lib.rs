//! 采集与只读进程共用的 Ability 装配与 Gateway 表面。

#![allow(clippy::type_complexity, clippy::new_ret_no_self)]

mod expose;
mod link_collector;
mod link_read;
mod surface;

pub use expose::{
    OPS_HTTP_INSTANCES, READ_AGENT_MCP_INSTANCES, ops_http_expose, read_agent_expose,
};
pub use link_collector::install_collector;
pub use link_read::install_read_agent;
pub use surface::{ConsoleGateway, HttpOpsGateway, McpReadGateway};
