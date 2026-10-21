// 自动生成，勿手改
use async_trait::async_trait;


#[derive(Debug, serde::Deserialize)]
pub struct LoadArgs {
    pub key: String,
    pub owner: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct StoreArgs {
    pub key: String,
    pub owner: String,
    pub value: String,
}


#[async_trait]
pub trait Handlers: Send + Sync {

    async fn load(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn store(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

}
