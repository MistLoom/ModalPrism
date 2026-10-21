use ability_support::{AbilityCtx, Result};
use serde_json::{Value, json};

pub async fn status(_ctx: &dyn AbilityCtx, _args: Value) -> Result<Value> {
    let bind = crate::runtime::bind()?;
    Ok(json!({ "listen": bind.push_listen }))
}
