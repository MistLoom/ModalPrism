//! 多语言代码生成目标。

mod py;
mod ts;

use std::path::Path;

use ability_support::AbilityPackageMeta;

pub enum CodegenTarget {
    Rust,
    Ts,
    Py,
}

impl CodegenTarget {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "rust" => Ok(Self::Rust),
            "ts" | "typescript" => Ok(Self::Ts),
            "py" | "python" => Ok(Self::Py),
            other => Err(format!("未知 codegen target: {other}")),
        }
    }
}

pub fn write_target(
    target: CodegenTarget,
    meta: &AbilityPackageMeta,
    out_dir: &Path,
) -> Result<(), String> {
    match target {
        CodegenTarget::Rust => crate::codegen::write_all(meta, out_dir),
        CodegenTarget::Ts => ts::write_all(meta, out_dir),
        CodegenTarget::Py => py::write_all(meta, out_dir),
    }
}
