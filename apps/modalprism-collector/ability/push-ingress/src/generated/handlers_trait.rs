// 自动生成，勿手改
use async_trait::async_trait;


#[derive(Debug, serde::Deserialize)]
pub struct StatusArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct Start_listenerArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct Stop_listenerArgs {

}


#[async_trait]
pub trait Handlers: Send + Sync {

    async fn status(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn start_listener(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn stop_listener(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

}
