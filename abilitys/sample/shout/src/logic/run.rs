//! 业务逻辑：run

use ability_echo::EchoExt;
use ability_support::{AbilityCtx, Error, Result};
use serde_json::{Value, json};

pub async fn run(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let echoed = ctx.echo().run(args).await?;
    let text = echoed
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::failed("echo 缺少 text"))?;
    Ok(json!({ "text": text.to_uppercase() }))
}
