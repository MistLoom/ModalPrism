use ability_support::{AbilityCtx, Result};
use serde_json::Value;

const BUILTIN: &str = "search";

pub async fn validate(_ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    collector_accessor_shared::validate(&crate::runtime::bind()?, BUILTIN, args).await
}
