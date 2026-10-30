// 自动生成，勿手改
use async_trait::async_trait;


#[derive(Debug, serde::Deserialize)]
pub struct LoadArgs {
#[serde(default)]
    pub format: String,
    pub path: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct EnvArgs {
#[serde(default)]
    pub content: String,
#[serde(default)]
    pub prefix: String,
}


#[async_trait]
pub trait Handlers: Send + Sync {

    async fn load(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn env(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

}
