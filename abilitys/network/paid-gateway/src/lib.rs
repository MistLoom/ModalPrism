mod bridge;
mod hot;
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
