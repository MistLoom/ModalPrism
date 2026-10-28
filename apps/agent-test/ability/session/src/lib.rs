//! Ability `session`。对外只暴露 `provider()`；Hub 与领域类型为 crate 内实现。
//!
//! 能力事件经 `ctx.emit`（Manifest.events）。用户事件走 Host 用户通道，不在本 crate 另建管道。

#![cfg_attr(not(test), allow(dead_code))]

mod ability;
mod error;
mod ext;
mod generated;
mod hub;
mod iter;
mod model;
mod query;
mod render;
mod run;
mod snapshot;
mod store;
mod sugar;

pub use ability::provider;

#[cfg(test)]
mod hub_tests;
