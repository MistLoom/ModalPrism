// 自动生成，勿手改
use async_trait::async_trait;


#[derive(Debug, serde::Deserialize)]
pub struct KindsArgs {
#[serde(default)]
    pub includeVariants: bool,
    pub rid: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct HasArgs {
    pub kind: String,
    pub rid: String,
#[serde(default)]
    pub variant: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct GetArgs {
    pub facet: String,
#[serde(default)]
    pub lazy: bool,
}

#[derive(Debug, serde::Deserialize)]
pub struct SliceArgs {
    pub span: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct ResolveArgs {
    pub identity: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct HistoryArgs {
    pub facet: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct LineageArgs {
#[serde(default)]
    pub depth: i64,
#[serde(default)]
    pub direction: String,
    pub facet: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct RelatedArgs {
#[serde(default)]
    pub direction: String,
#[serde(default)]
    pub rel: String,
    pub rid: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct QueryArgs {
    pub query: serde_json::Value,
}

#[derive(Debug, serde::Deserialize)]
pub struct ChangesArgs {
    pub after: i64,
#[serde(default)]
    pub limit: i64,
}


#[async_trait]
pub trait Handlers: Send + Sync {

    async fn kinds(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn has(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn get(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn slice(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn resolve(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn history(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn lineage(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn related(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn query(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn changes(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

}
