// 自动生成，勿手改
use async_trait::async_trait;


#[derive(Debug, serde::Deserialize)]
pub struct BuildArgs {
#[serde(default)]
    pub assistant: String,
#[serde(default)]
    pub skill_path: String,
#[serde(default)]
    pub skill_paths: Vec<String>,
#[serde(default)]
    pub skill_root: String,
#[serde(default)]
    pub system: String,
#[serde(default)]
    pub user: String,
}


#[async_trait]
pub trait Handlers: Send + Sync {

    async fn build(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

}
