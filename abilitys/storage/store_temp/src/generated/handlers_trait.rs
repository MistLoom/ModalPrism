// 自动生成，勿手改
use async_trait::async_trait;


#[derive(Debug, serde::Deserialize)]
pub struct GetArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct PutArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct MergeArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct DeleteArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct ExistsArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct ListArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct PurgeArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct StatArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct Blob_getArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct Blob_putArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct Blob_writeArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct AcquireArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct ReleaseArgs {

}


#[async_trait]
pub trait Handlers: Send + Sync {

    async fn get(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn put(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn merge(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn delete(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn exists(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn list(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn purge(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn stat(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn blob_get(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn blob_put(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn blob_write(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn acquire(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn release(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

}
