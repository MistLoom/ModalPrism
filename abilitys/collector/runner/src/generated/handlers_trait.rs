// 自动生成，勿手改
use async_trait::async_trait;


#[derive(Debug, serde::Deserialize)]
pub struct Run_taskArgs {
    pub task: serde_json::Value,
}


#[async_trait]
pub trait Handlers: Send + Sync {

    async fn run_task(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

}
