// 自动生成，勿手改
use async_trait::async_trait;


#[derive(Debug, serde::Deserialize)]
pub struct ReadArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct WriteArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct Patch_frontmatterArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct AppendArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct HeadingsArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct ParseArgs {

}


#[async_trait]
pub trait Handlers: Send + Sync {

    async fn read(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn write(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn patch_frontmatter(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn append(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn headings(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn parse(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

}
