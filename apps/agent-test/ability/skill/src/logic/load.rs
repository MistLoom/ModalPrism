//! 业务逻辑：load

use ability_support::{AbilityCtx, Error, Result};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::loader::SkillLoader;

#[derive(Deserialize)]
struct PathArgs {
    path: Option<String>,
    root: Option<String>,
}

pub async fn load(_ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let a: PathArgs =
        serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
    let path = a.path.ok_or_else(|| Error::invalid_args("缺少 path"))?;
    let skill = SkillLoader::new()
        .load_dir(path)
        .map_err(|e| Error::failed(e.to_string()))?;
    Ok(json!({
        "name": skill.meta.name,
        "description": skill.meta.description,
        "body": skill.body
    }))
}
