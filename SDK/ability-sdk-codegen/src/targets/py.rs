//! 生成 Python manifest / 路由表。

use std::path::Path;

use ability_support::AbilityPackageMeta;

use crate::{write_generated, write_scaffold_once};

pub fn write_all(meta: &AbilityPackageMeta, out_dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let decl = &meta.decl;
    let methods = decl
        .methods
        .iter()
        .map(|m| format!("    \"{}\",", m.id))
        .collect::<Vec<_>>()
        .join("\n");

    let manifest = format!(
        r#"# @generated
ABILITY_ID = "{id}"
FRAMEWORK_VERSION = "{fw}"


def schema_version(fw: str) -> str:
    i = fw.find("@")
    return fw[i + 1 :] if i >= 0 else fw


METHOD_IDS = (
{methods}
)


def assert_framework_version() -> None:
    if schema_version(FRAMEWORK_VERSION) != "{fw_base}":
        raise RuntimeError("framework_version schema 与当前框架不匹配")
"#,
        id = decl.id,
        fw = decl.framework_version,
        fw_base = ability_support::FRAMEWORK_VERSION,
        methods = methods,
    );

    write_generated(&out_dir.join("manifest.py"), &manifest)?;

    let route_lines = decl
        .methods
        .iter()
        .map(|m| format!("    \"{}\": \"{}\",", m.id, m.id))
        .collect::<Vec<_>>()
        .join("\n");
    let routes = format!(
        r"# @generated
ROUTES = {{
{route_lines}
}}
",
        route_lines = route_lines,
    );
    write_generated(&out_dir.join("routes.py"), &routes)?;

    write_scaffold_once(
        &out_dir.join("__init__.py"),
        "from .manifest import ABILITY_ID, FRAMEWORK_VERSION, METHOD_IDS\n",
    )?;
    Ok(())
}
