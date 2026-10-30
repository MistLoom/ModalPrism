// 自动生成，勿手改
use std::sync::Arc;

use async_trait::async_trait;

use super::handlers_trait::Handlers;

pub struct Bridge;

#[async_trait]
impl Handlers for Bridge {

    async fn read(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
        crate::logic::read::read(ctx, args).await
    }

    async fn write(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
        crate::logic::write::write(ctx, args).await
    }

    async fn patch_frontmatter(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
        crate::logic::patch_frontmatter::patch_frontmatter(ctx, args).await
    }

    async fn append(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
        crate::logic::append::append(ctx, args).await
    }

    async fn headings(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
        crate::logic::headings::headings(ctx, args).await
    }

    async fn parse(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
        crate::logic::parse::parse(ctx, args).await
    }

}

pub fn instance() -> Arc<dyn Handlers> {
    Arc::new(Bridge)
}
