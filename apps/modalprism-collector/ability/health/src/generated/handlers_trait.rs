// 自动生成，勿手改
use async_trait::async_trait;


#[derive(Debug, serde::Deserialize)]
pub struct EvaluateArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct StatusArgs {
    pub source: String,
}


#[async_trait]
pub trait Handlers: Send + Sync {

    async fn evaluate(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn status(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

}
