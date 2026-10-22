//! 保守写入策略：generated 可覆盖；logic / bin 仅 scaffold。

use std::fs;
use std::path::Path;

/// 始终覆盖（仅用于 `src/generated/`）。
pub fn write_generated(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(path, content).map_err(|e| e.to_string())
}

/// 文件不存在时写入；已存在则跳过。
pub fn write_scaffold_once(path: &Path, content: &str) -> Result<bool, String> {
    if path.is_file() {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(path, content).map_err(|e| e.to_string())?;
    Ok(true)
}

/// 向 `mod.rs` 追加缺失的 `pub mod xxx;` 行，不修改已有内容。
pub fn ensure_mod_declarations(mod_path: &Path, modules: &[&str]) -> Result<(), String> {
    let existing = if mod_path.is_file() {
        fs::read_to_string(mod_path).map_err(|e| e.to_string())?
    } else {
        String::new()
    };

    let mut lines: Vec<String> = if existing.is_empty() {
        vec![
            "//! 业务能力逻辑。勿写入声明或协议相关代码。".into(),
            String::new(),
        ]
    } else {
        existing.lines().map(String::from).collect()
    };

    let mut changed = false;
    for name in modules {
        let decl = format!("pub mod {name};");
        if !lines.iter().any(|l| l.trim() == decl) {
            lines.push(decl);
            changed = true;
        }
    }

    if existing.is_empty() || changed {
        if !lines.last().map(|l| l.is_empty()).unwrap_or(true) {
            lines.push(String::new());
        }
        if let Some(parent) = mod_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(mod_path, lines.join("\n")).map_err(|e| e.to_string())?;
    }
    Ok(())
}
