//! 业务逻辑：patch_frontmatter

use ability_support::{AbilityCtx, Error, Result};
use serde_json::{Value, json};

use super::helpers::{PatchArgs, read_doc, write_doc};

pub async fn patch_frontmatter(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let a: PatchArgs =
        serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
    let (mut fm, body) = read_doc(ctx, &a.key, a.owner.as_deref()).await?;
    match (&mut fm, a.patch) {
        (Value::Object(base), Value::Object(p)) => {
            for (k, v) in p {
                base.insert(k, v);
            }
        }
        (_, p) => fm = p,
    }
    write_doc(ctx, a.key, a.owner, fm.clone(), body).await?;
    Ok(json!({ "frontmatter": fm }))
}
