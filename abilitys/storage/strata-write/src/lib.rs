#![allow(dead_code, non_camel_case_types, unused_imports)]
//! Ability `strata_write`

mod ability;
mod generated;
mod logic;

pub use ability::provider;

use ability_support::client;

#[client(id = "strata_write", ext = strata_write)]
pub trait StrataWrite {
    async fn apply(&self, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;
}
