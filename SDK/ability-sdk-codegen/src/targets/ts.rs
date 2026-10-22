//! 生成 TypeScript manifest / 路由表。

use std::path::Path;

use ability_support::AbilityPackageMeta;

use crate::{write_generated, write_scaffold_once};

pub fn write_all(meta: &AbilityPackageMeta, out_dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let decl = &meta.decl;
    let methods = decl
        .methods
        .iter()
        .map(|m| format!("  \"{}\",", m.id))
        .collect::<Vec<_>>()
        .join("\n");

    let manifest = format!(
        r#"// @generated
export const ABILITY_ID = "{id}";
export const FRAMEWORK_VERSION = "{fw}";

export const METHOD_IDS = [
{methods}
] as const;

export function schemaVersion(fw: string) {{
  const i = fw.indexOf("@");
  return i >= 0 ? fw.slice(i + 1) : fw;
}}

export function assertFrameworkVersion() {{
  const base = "{fw_base}";
  if (schemaVersion(FRAMEWORK_VERSION) !== base) {{
    throw new Error("framework_version schema mismatch");
  }}
}}
"#,
        id = decl.id,
        fw = decl.framework_version,
        fw_base = ability_support::FRAMEWORK_VERSION,
        methods = methods,
    );

    write_generated(&out_dir.join("manifest.ts"), &manifest)?;

    let entries = decl
        .methods
        .iter()
        .map(|m| format!("  \"{}\": \"{}\",", m.id, m.id))
        .collect::<Vec<_>>()
        .join("\n");
    let routes = format!(
        r#"// @generated
import {{ METHOD_IDS }} from "./manifest.js";

export const ROUTES: Record<string, string> = {{
{entries}
}};
"#,
        entries = entries,
    );
    write_generated(&out_dir.join("routes.ts"), &routes)?;

    write_scaffold_once(
        &out_dir.join("index.ts"),
        r#"export * from "./manifest.js";
export * from "./routes.js";
"#,
    )?;
    Ok(())
}
