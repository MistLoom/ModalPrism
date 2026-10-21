// 自动生成，勿手改
use async_trait::async_trait;


#[derive(Debug, serde::Deserialize)]
pub struct TickArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct TriggerArgs {
    pub source: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct NextArgs {
    pub source: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct Source_statusArgs {
    pub source: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct Health_snapshotArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct Health_commitArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct ShutdownArgs {

}


#[async_trait]
pub trait Handlers: Send + Sync {

    async fn tick(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn trigger(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn next(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn source_status(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn health_snapshot(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn health_commit(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn shutdown(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

}
