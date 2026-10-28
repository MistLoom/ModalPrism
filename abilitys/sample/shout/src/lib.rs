//! Ability `app.shout`：经 `algo.echo` 的 client 调用，不依赖对方 Provider。

mod generated;
mod logic;

pub use generated::provider::provider;
