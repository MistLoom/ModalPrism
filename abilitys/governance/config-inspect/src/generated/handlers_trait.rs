// 自动生成，勿手改
use async_trait::async_trait;


#[derive(Debug, serde::Deserialize)]
pub struct ValidateArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct DiffArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct ExplainArgs {
    pub module: String,
}


#[async_trait]
pub trait Handlers: Send + Sync {

    async fn validate(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn diff(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn explain(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

}
