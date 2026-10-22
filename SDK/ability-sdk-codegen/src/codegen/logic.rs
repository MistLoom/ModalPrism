//! 为 `src/logic/` 生成空实现桩（仅当文件不存在时）。

use std::path::Path;

use ability_support::{AbilityDecl, AbilityPackageMeta};

use super::handlers::method_fn_name;
use super::write::{ensure_mod_declarations, write_scaffold_once};

pub fn scaffold_logic(root: &Path, meta: &AbilityPackageMeta) -> Result<(), String> {
    let logic_dir = root.join("src").join("logic");
    let mod_path = logic_dir.join("mod.rs");

    let module_names: Vec<String> = meta
        .decl
        .methods
        .iter()
        .map(|m| AbilityDecl::logic_module_name(&m.id))
        .collect();

    let refs: Vec<&str> = module_names.iter().map(String::as_str).collect();
    ensure_mod_declarations(&mod_path, &refs)?;

    for m in &meta.decl.methods {
        let mod_name = AbilityDecl::logic_module_name(&m.id);
        let fn_name = method_fn_name(&m.id);
        let file = logic_dir.join(format!("{mod_name}.rs"));
        let stub = format!(
            r#"//! 业务逻辑：{method_id}
//!
//! 仅实现业务，勿引用 generated 或声明类型。

use ability_support::{{AbilityCtx, Result}};
use serde_json::Value;

pub async fn {fn_name}(_ctx: &dyn AbilityCtx, _args: Value) -> Result<Value> {{
    Ok(serde_json::json!({{ "ok": true }}))
}}
"#,
            method_id = m.id,
            fn_name = fn_name,
        );
        write_scaffold_once(&file, &stub)?;
    }

    Ok(())
}
