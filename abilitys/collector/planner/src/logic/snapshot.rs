use ability_support::{AbilityCtx, Result};

pub async fn from_ctx(ctx: &dyn AbilityCtx) -> Result<std::sync::Arc<config::Snapshot>> {
    collector_ports::applied(ctx).await
}
