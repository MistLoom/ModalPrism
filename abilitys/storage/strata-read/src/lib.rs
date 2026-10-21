//! Ability `strata_read`

mod logic;
mod runtime;
#[allow(dead_code, unused_imports)]
mod generated;

pub use generated::provider::provider;

pub mod install {
    use ability_support::Result;
    use serde_json::Value;

    pub async fn prime(config: Value) -> Result<()> {
        crate::runtime::prime(config).await
    }
}

use ability_support::client;

#[client(id = "strata_read", ext = strata_read)]
pub trait StrataRead {
    async fn kinds(&self, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;
    async fn get(&self, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;
    async fn resolve(&self, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;
}
