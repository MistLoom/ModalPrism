//! Ability `dispatch`：LLM tool_calls → catalog / ctx.call。

mod bridge;
mod generated;
mod logic;
mod map;

pub use generated::provider::provider;
