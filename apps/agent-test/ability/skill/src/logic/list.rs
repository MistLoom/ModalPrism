//! 业务逻辑：list

use ability_support::{AbilityCtx, Error, Result};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::catalog::SkillCatalog;

#[derive(Deserialize)]
struct PathArgs {
    path: Option<String>,
    root: Option<String>,
}

pub async fn list(_ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let a: PathArgs =
        serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
    let root = a.root.ok_or_else(|| Error::invalid_args("缺少 root"))?;
    let mut cat = SkillCatalog::new();
    cat.load_root(root)
        .map_err(|e| Error::failed(e.to_string()))?;
    Ok(json!({
        "catalog": cat.render_catalog(),
        "count": cat.len()
    }))
}
