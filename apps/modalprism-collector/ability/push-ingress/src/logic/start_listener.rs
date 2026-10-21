use ability_support::{AbilityCtx, Result};
use serde_json::Value;

pub async fn start_listener(ctx: &dyn AbilityCtx, _args: Value) -> Result<Value> {
    let bind = crate::runtime::bind()?;
    crate::service::serve(bind, ctx.retain()).await?;
    Ok(Value::Null)
}
