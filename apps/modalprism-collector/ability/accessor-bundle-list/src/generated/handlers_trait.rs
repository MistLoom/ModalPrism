// 自动生成，勿手改
use async_trait::async_trait;


#[derive(Debug, serde::Deserialize)]
pub struct ValidateArgs {
#[serde(default)]
    pub spec: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct AccessArgs {

}


#[async_trait]
pub trait Handlers: Send + Sync {

    async fn validate(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn access(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

}
