// 自动生成，勿手改
use async_trait::async_trait;


#[derive(Debug, serde::Deserialize)]
pub struct DefsArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct RunArgs {
    pub calls: Vec<String>,
#[serde(default)]
    pub continue_on_error: bool,
}


#[async_trait]
pub trait Handlers: Send + Sync {

    async fn defs(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn run(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

}
