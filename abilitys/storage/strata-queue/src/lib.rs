//! Ability `strata_queue`

mod ability;
mod generated;
mod logic;

pub use ability::provider;

use ability_support::client;

#[client(id = "strata_queue", ext = strata_queue)]
pub trait StrataQueue {
    async fn enqueue(&self, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;
    async fn lease(&self, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;
    async fn nack(&self, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;
}
