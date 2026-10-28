//! 业务逻辑：build

use ability_support::{AbilityCtx, Error, Result};
use ai_skill::SkillExt;
use serde::Deserialize;
use serde_json::Value;

use crate::builder::PromptBuilder;

#[derive(Deserialize)]
struct BuildArgs {
    system: Option<String>,
    user: Option<String>,
    assistant: Option<String>,
    skill_path: Option<String>,
    #[serde(default)]
    skill_paths: Vec<String>,
    skill_root: Option<String>,
}

fn inject_loaded(b: &mut PromptBuilder, loaded: &Value) -> Result<()> {
    let name = loaded
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::failed("skill 缺少 name"))?;
    let description = loaded
        .get("description")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::failed("skill 缺少 description"))?;
    let body = loaded
        .get("body")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::failed("skill 缺少 body"))?;
    b.skill(name, description, body);
    Ok(())
}

pub async fn build(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let a: BuildArgs =
        serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
    let mut b = PromptBuilder::new();
    if let Some(s) = a.system {
        b.system(s);
    }
    if let Some(root) = a.skill_root {
        let listed = ctx
            .skill()
            .list(serde_json::json!({ "root": root }))
            .await?;
        let catalog = listed
            .get("catalog")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::failed("skill 缺少 catalog"))?;
        b.skill_catalog(catalog);
    }
    let mut paths = a.skill_paths;
    if let Some(path) = a.skill_path {
        paths.push(path);
    }
    for path in paths {
        let loaded = ctx
            .skill()
            .load(serde_json::json!({ "path": path }))
            .await?;
        inject_loaded(&mut b, &loaded)?;
    }
    if let Some(s) = a.user {
        b.user(s);
    }
    if let Some(s) = a.assistant {
        b.assistant(s);
    }
    let msgs = b.build();
    serde_json::to_value(msgs).map_err(|e| Error::failed(e.to_string()))
}
