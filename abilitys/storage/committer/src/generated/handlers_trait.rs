// 自动生成，勿手改
use async_trait::async_trait;


#[derive(Debug, serde::Deserialize)]
pub struct FlushArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct MaintainArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct RecoverArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct StatusArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct Hotlist_applyArgs {

}


#[async_trait]
pub trait Handlers: Send + Sync {

    async fn flush(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn maintain(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn recover(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn status(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn hotlist_apply(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

}
