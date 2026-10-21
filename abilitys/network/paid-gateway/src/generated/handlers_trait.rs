// 自动生成，勿手改
use async_trait::async_trait;


#[derive(Debug, serde::Deserialize)]
pub struct ReserveArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct CompleteArgs {
    pub receiptId: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct StatusArgs {
    pub receiptId: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct CallArgs {
    pub body: String,
    pub endpoint: String,
    pub headers: Vec<Vec<String>>,
    pub params: serde_json::Value,
#[serde(default)]
    pub policy: serde_json::Value,
}


#[async_trait]
pub trait Handlers: Send + Sync {

    async fn reserve(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn complete(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn status(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn call(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

}
