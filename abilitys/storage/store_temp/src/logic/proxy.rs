//! store 代理：temp 空间

use ability_support::{AbilityCtx, MethodId, Result};
use serde_json::{Value, json};

fn fill(mut args: Value, ctx: &dyn AbilityCtx) -> Value {
    if let Value::Object(m) = &mut args {
        m.insert("space".into(), json!("temp"));
        m.entry("owner")
            .or_insert_with(|| json!(ability_store::caller_owner(ctx)));
    }
    args
}

pub async fn forward(ctx: &dyn AbilityCtx, method: &str, args: Value) -> Result<Value> {
    let mid =
        MethodId::new(method).map_err(|e| ability_support::Error::invalid_args(e.to_string()))?;
    ctx.call("store", &mid, fill(args, ctx)).await
}
