//! Ability `strata_read`

mod ability;
#[allow(dead_code, unused_imports)]
mod generated;
mod logic;

pub use ability::provider;

use ability_support::client;

#[client(id = "strata_read", ext = strata_read)]
pub trait StrataRead {
    async fn kinds(&self, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;
    async fn get(&self, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;
    async fn resolve(&self, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;
}
