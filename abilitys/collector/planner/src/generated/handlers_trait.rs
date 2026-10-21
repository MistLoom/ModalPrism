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


#[async_trait]
pub trait Handlers: Send + Sync {

    async fn tick(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn trigger(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn next(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

}
