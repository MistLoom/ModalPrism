//! Ability `config`：`load` / `env`。组合根读盘不在本 crate。

mod format;
mod generated;
mod load;
mod logic;

pub use generated::provider::provider;
