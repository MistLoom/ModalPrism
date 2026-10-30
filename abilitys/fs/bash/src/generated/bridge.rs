// 自动生成，勿手改
use std::sync::Arc;

use async_trait::async_trait;

use super::handlers_trait::Handlers;

pub struct Bridge;

#[async_trait]
impl Handlers for Bridge {

    async fn run(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value> {
        crate::logic::run::run(ctx, args).await
    }

}

pub fn instance() -> Arc<dyn Handlers> {
    Arc::new(Bridge)
}
