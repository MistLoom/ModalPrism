//! 业务逻辑：read

use ability_support::{AbilityCtx, Error, Result};
use serde_json::{Value, json};

use super::helpers::{KeyArgs, headings_of, read_doc};

pub async fn read(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let a: KeyArgs =
        serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
    let (fm, body) = read_doc(ctx, &a.key, a.owner.as_deref()).await?;
    Ok(json!({
        "frontmatter": fm,
        "body": body,
        "headings": headings_of(&body)
    }))
}
