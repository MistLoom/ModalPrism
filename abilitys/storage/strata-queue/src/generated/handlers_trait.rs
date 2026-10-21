// 自动生成，勿手改
use async_trait::async_trait;


#[derive(Debug, serde::Deserialize)]
pub struct EnqueueArgs {
    pub tasks: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct LeaseArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct ExtendArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct NackArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct Lease_idArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct Release_leasesArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct ActiveArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct Lease_readyArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct Lease_followsArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct Leased_countArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct ApplyArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct Accepted_cursorArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct Visible_cursorArgs {

}


#[async_trait]
pub trait Handlers: Send + Sync {

    async fn enqueue(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn lease(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn extend(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn nack(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn lease_id(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn release_leases(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn active(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn lease_ready(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn lease_follows(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn leased_count(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn apply(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn accepted_cursor(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn visible_cursor(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

}
