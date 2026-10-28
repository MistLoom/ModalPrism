//! 业务逻辑：run

use ability_support::{AbilityCtx, Error, Result};
use serde::Deserialize;
use serde_json::{Value, json};

const STATE_KEY: &str = "count";

#[derive(Deserialize)]
struct Args {
    #[serde(default = "one")]
    delta: i64,
}

fn one() -> i64 {
    1
}

pub async fn run(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let parsed: Args =
        serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
    let current = match ctx.local_get(STATE_KEY).await? {
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
        Some(_) => return Err(Error::failed("count 非数字")),
        None => 0,
    };
    let next = current + parsed.delta;
    ctx.local_set(STATE_KEY, json!(next)).await?;
    Ok(json!({ "count": next }))
}
