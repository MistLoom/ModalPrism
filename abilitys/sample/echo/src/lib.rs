//! Ability `algo.echo`

mod generated;
mod logic;

use ability_support::client;

pub use generated::provider::provider;

#[client(id = "algo.echo", ext = echo)]
pub trait Echo {
    async fn run(&self, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;
}
