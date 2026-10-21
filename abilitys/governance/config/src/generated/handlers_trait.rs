// 自动生成，勿手改
use async_trait::async_trait;


#[derive(Debug, serde::Deserialize)]
pub struct LoadArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct GenerationArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct LayoutArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct AppliedArgs {

}


#[async_trait]
pub trait Handlers: Send + Sync {

    async fn load(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn generation(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn layout(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn applied(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

}
