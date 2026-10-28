//! 业务逻辑：write

use ability_support::{AbilityCtx, Error, Result};
use serde_json::{Value, json};

use super::helpers::{WriteArgs, read_doc, write_doc};

pub async fn write(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let a: WriteArgs =
        serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
    let (mut fm, mut body) = read_doc(ctx, &a.key, a.owner.as_deref()).await?;
    if let Some(f) = a.frontmatter {
        fm = f;
    }
    if let Some(b) = a.body {
        body = b;
    }
    write_doc(ctx, a.key, a.owner, fm, body).await?;
    Ok(json!({ "ok": true }))
}
