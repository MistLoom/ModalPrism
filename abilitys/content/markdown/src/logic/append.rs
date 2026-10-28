//! 业务逻辑：append

use ability_support::{AbilityCtx, Error, Result};
use serde_json::{Value, json};

use super::helpers::{AppendArgs, read_doc, write_doc};

pub async fn append(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let a: AppendArgs =
        serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
    let (fm, mut body) = read_doc(ctx, &a.key, a.owner.as_deref()).await?;
    body.push_str(&a.text);
    write_doc(ctx, a.key, a.owner, fm, body).await?;
    Ok(json!({ "ok": true }))
}
