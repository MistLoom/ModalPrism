//! Ability `store`：二进制与结构数据的统一落盘门面。

mod ability;
mod client;
mod codec;
mod engine;
mod generated;
mod loc;
mod types;

pub use ability::provider;
pub use client::{Store, StoreExt, StoreHandle, caller_owner};
pub use types::*;
