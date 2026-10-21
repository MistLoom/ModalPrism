//! Ability `strata_write`

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

#[client(id = "strata_write", ext = strata_write)]
pub trait StrataWrite {
    async fn apply(&self, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;
}
