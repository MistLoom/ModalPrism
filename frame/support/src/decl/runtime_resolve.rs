//! Guest 固定入口与运行时解析（无 `runtime` 块）。

use std::path::{Path, PathBuf};

use super::ability::RuntimeKind;

/// Node guest 唯一入口（相对能力根）。
pub const GUEST_SERVE_MJS: &str = "src/serve.mjs";

/// Python guest 唯一入口（相对能力根）。
pub const GUEST_SERVE_PY: &str = "src/serve.py";

/// Cangjie guest 唯一入口（相对能力根）。
pub const GUEST_SERVE_CJ: &str = "src/serve.cj";

/// Cangjie guest 构建清单（相对能力根）。
pub const GUEST_CJPM_TOML: &str = "cjpm.toml";

/// 从 `framework_version` 与能力根目录解析运行时。
///
/// - `1` / `rust@1`：Rust（默认，需 `Cargo.toml`）
/// - `node@1` / `ts@1`：Node guest
/// - `python@1` / `py@1`：Python guest
/// - `cangjie@1` / `cj@1`：Cangjie guest
pub fn resolve_runtime_kind(root: &Path, framework_version: &str) -> Result<RuntimeKind, String> {
    let (explicit, schema) = parse_framework_tag(framework_version)?;
    super::version::validate_schema_version(schema)?;
    match explicit {
        Some(kind) => Ok(kind),
        None => infer_runtime_kind(root),
    }
}

/// 固定 guest 入口路径。
pub fn guest_entry_path(root: &Path, kind: RuntimeKind) -> Result<PathBuf, String> {
    match kind {
        RuntimeKind::Rust => Err("Rust 能力无 guest 入口".into()),
        RuntimeKind::Node => {
            let p = root.join(GUEST_SERVE_MJS);
            if !p.is_file() {
                return Err(format!("缺少 guest 入口 {GUEST_SERVE_MJS}"));
            }
            Ok(p)
        }
        RuntimeKind::Python => {
            let p = root.join(GUEST_SERVE_PY);
            if !p.is_file() {
                return Err(format!("缺少 guest 入口 {GUEST_SERVE_PY}"));
            }
            Ok(p)
        }
        RuntimeKind::Cangjie => {
            let p = root.join(GUEST_SERVE_CJ);
            if !p.is_file() {
                return Err(format!("缺少 guest 入口 {GUEST_SERVE_CJ}"));
            }
            Ok(p)
        }
    }
}

pub fn lint_runtime_artifacts(root: &Path, kind: RuntimeKind, errors: &mut Vec<String>) {
    match kind {
        RuntimeKind::Rust => {
            if !root.join("Cargo.toml").is_file() {
                errors.push("Rust 能力需要 Cargo.toml".into());
            }
        }
        RuntimeKind::Node => {
            if !root.join("package.json").is_file() {
                errors.push("Node guest 需要 package.json".into());
            }
            if !root.join(GUEST_SERVE_MJS).is_file() {
                errors.push(format!("Node guest 需要 {GUEST_SERVE_MJS}"));
            }
        }
        RuntimeKind::Python => {
            if !root.join("pyproject.toml").is_file() {
                errors.push("Python guest 需要 pyproject.toml".into());
            }
            if !root.join(GUEST_SERVE_PY).is_file() {
                errors.push(format!("Python guest 需要 {GUEST_SERVE_PY}"));
            }
        }
        RuntimeKind::Cangjie => {
            if !root.join(GUEST_CJPM_TOML).is_file() {
                errors.push(format!("Cangjie guest 需要 {GUEST_CJPM_TOML}"));
            }
            if !root.join(GUEST_SERVE_CJ).is_file() {
                errors.push(format!("Cangjie guest 需要 {GUEST_SERVE_CJ}"));
            }
        }
    }
}

/// 解析 `framework_version`：`(显式运行时, schema 段)`；无 `@` 时运行时为 `None`（由目录推断）。
pub fn parse_framework_tag(framework_version: &str) -> Result<(Option<RuntimeKind>, &str), String> {
    if let Some((tag, schema)) = framework_version.split_once('@') {
        let kind = match tag {
            "rust" => RuntimeKind::Rust,
            "node" | "ts" => RuntimeKind::Node,
            "python" | "py" => RuntimeKind::Python,
            "cangjie" | "cj" => RuntimeKind::Cangjie,
            other => return Err(format!("未知 framework_version 运行时前缀: {other}")),
        };
        Ok((Some(kind), schema))
    } else {
        Ok((None, framework_version))
    }
}

fn infer_runtime_kind(root: &Path) -> Result<RuntimeKind, String> {
    let has_cargo = root.join("Cargo.toml").is_file();
    let has_mjs = root.join(GUEST_SERVE_MJS).is_file();
    let has_py = root.join(GUEST_SERVE_PY).is_file();
    let has_cj = root.join(GUEST_SERVE_CJ).is_file();
    let manifest_count = [has_mjs, has_py, has_cj].iter().filter(|b| **b).count();
    if has_cargo {
        if manifest_count > 0 {
            return Err(
                "同时存在 Cargo.toml 与 guest 入口，请在 framework_version 使用 rust@1 / node@1 / python@1 / cangjie@1"
                    .into(),
            );
        }
        return Ok(RuntimeKind::Rust);
    }
    if manifest_count > 1 {
        return Err(
            "同时存在多个 guest 入口（src/serve.mjs / src/serve.py / src/serve.cj），请显写 framework_version"
                .into(),
        );
    }
    if has_mjs {
        return Ok(RuntimeKind::Node);
    }
    if has_py {
        return Ok(RuntimeKind::Python);
    }
    if has_cj {
        return Ok(RuntimeKind::Cangjie);
    }
    if root.join("package.json").is_file() {
        return Ok(RuntimeKind::Node);
    }
    if root.join("pyproject.toml").is_file() {
        return Ok(RuntimeKind::Python);
    }
    if root.join(GUEST_CJPM_TOML).is_file() {
        return Ok(RuntimeKind::Cangjie);
    }
    Err("无法推断运行时：需要 Cargo.toml、src/serve.mjs、src/serve.py 或 src/serve.cj".into())
}
