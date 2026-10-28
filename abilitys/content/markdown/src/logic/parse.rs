//! 业务逻辑：parse

use ability_support::{AbilityCtx, Error, Result};
use serde_json::{Value, json};

use super::helpers::{ParseArgs, headings_of, parse_md};

pub async fn parse(_ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let a: ParseArgs =
        serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
    let doc = parse_md(&a.text);
    let body = doc.get("body").and_then(Value::as_str).unwrap_or("");
    Ok(json!({
        "frontmatter": doc.get("frontmatter"),
        "body": body,
        "headings": headings_of(body)
    }))
}
