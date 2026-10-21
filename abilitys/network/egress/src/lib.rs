mod ability;
mod hot;
mod logic;
#[allow(dead_code, unused_imports)]
mod generated;

pub use ability::provider;

use ability_support::client;

#[client(id = "egress", ext = egress)]
pub trait Egress {
    async fn check(&self, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;
}
