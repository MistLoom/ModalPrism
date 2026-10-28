//! Markdown 文档读写辅助。

use ability_store::{GetArgs, GetOut, PutArgs};
use ability_store_persist::PersistExt;
use ability_support::{AbilityCtx, Result};
use serde::Deserialize;
use serde_json::{Map, Value, json};

#[derive(Deserialize)]
pub struct KeyArgs {
    pub key: String,
    #[serde(default)]
    pub owner: Option<String>,
}

#[derive(Deserialize)]
pub struct WriteArgs {
    pub key: String,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub frontmatter: Option<Value>,
    #[serde(default)]
    pub body: Option<String>,
}

#[derive(Deserialize)]
pub struct PatchArgs {
    pub key: String,
    #[serde(default)]
    pub owner: Option<String>,
    pub patch: Value,
}

#[derive(Deserialize)]
pub struct AppendArgs {
    pub key: String,
    #[serde(default)]
    pub owner: Option<String>,
    pub text: String,
}

#[derive(Deserialize)]
pub struct ParseArgs {
    pub text: String,
}

pub fn headings_of(body: &str) -> Vec<Value> {
    body.lines()
        .filter_map(|line| {
            let t = line.trim_start();
            let n = t.chars().take_while(|c| *c == '#').count();
            if n >= 1 && n <= 6 && t.as_bytes().get(n) == Some(&b' ') {
                Some(json!({ "level": n, "text": t[n + 1..].trim() }))
            } else {
                None
            }
        })
        .collect()
}

pub fn parse_md(text: &str) -> Value {
    let mut map = Map::new();
    if let Some(rest) = text.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---\n") {
            let fm = serde_yaml::from_str(&rest[..end]).unwrap_or(Value::Null);
            map.insert("frontmatter".into(), fm);
            map.insert("body".into(), json!(&rest[end + 5..]));
            return Value::Object(map);
        }
    }
    map.insert("frontmatter".into(), Value::Null);
    map.insert("body".into(), json!(text));
    Value::Object(map)
}

pub fn as_doc(v: &Value) -> (Value, String) {
    match v {
        Value::Object(m) => (
            m.get("frontmatter").cloned().unwrap_or(Value::Null),
            m.get("body")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        ),
        Value::String(s) => (Value::Null, s.clone()),
        _ => (Value::Null, String::new()),
    }
}

pub async fn read_doc(
    ctx: &dyn AbilityCtx,
    key: &str,
    owner: Option<&str>,
) -> Result<(Value, String)> {
    let out: GetOut = ctx
        .persist()
        .get(GetArgs {
            owner: owner.map(|s| s.to_string()),
            space: None,
            key: key.into(),
            codec: Some("md".into()),
        })
        .await?;
    if !out.exists {
        return Ok((Value::Null, String::new()));
    }
    Ok(as_doc(&out.value.unwrap_or(Value::Null)))
}

pub async fn write_doc(
    ctx: &dyn AbilityCtx,
    key: String,
    owner: Option<String>,
    fm: Value,
    body: String,
) -> Result<()> {
    ctx.persist()
        .put(PutArgs {
            owner,
            space: None,
            key,
            value: json!({ "frontmatter": fm, "body": body }),
            codec: Some("md".into()),
        })
        .await?;
    Ok(())
}
