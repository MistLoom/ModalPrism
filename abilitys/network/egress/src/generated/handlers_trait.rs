// 自动生成，勿手改
use async_trait::async_trait;


#[derive(Debug, serde::Deserialize)]
pub struct CheckArgs {
    pub url: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct StatusArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct Governance_domainsArgs {

}

#[derive(Debug, serde::Deserialize)]
pub struct Reset_circuitArgs {
#[serde(default)]
    pub confirm: bool,
    pub host: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct Http_requestArgs {
    pub body: String,
    pub headers: Vec<Vec<String>>,
    pub method: String,
#[serde(default)]
    pub modulePolicy: serde_json::Value,
    pub url: String,
}


#[async_trait]
pub trait Handlers: Send + Sync {

    async fn check(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn status(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn governance_domains(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn reset_circuit(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

    async fn http_request(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;

}
