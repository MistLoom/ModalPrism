//! Ability 声明文件代码生成。

mod codegen;
mod targets;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub use codegen::CodegenOptions;
pub use codegen::{ensure_mod_declarations, write_generated, write_scaffold_once};
pub use targets::{CodegenTarget, write_target};
pub use ability_support::FRAMEWORK_VERSION;

/// 从能力根目录生成代码到 `src/generated/`。
pub fn generate(root: impl AsRef<Path>, out_dir: impl AsRef<Path>) -> Result<(), String> {
    let root = root.as_ref();
    let out_dir = out_dir.as_ref();
    let meta = ability_support::load_ability_package(root)?;
    fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    codegen::write_all(&meta, out_dir)?;
    println!("cargo:rerun-if-changed={}", root.join("ability.yml").display());
    for kind in meta.protocols.iter() {
        let info = meta.root.join(kind.dir_name()).join("info.yml");
        println!("cargo:rerun-if-changed={}", info.display());
    }
    Ok(())
}

/// build.rs 便捷入口：能力根 = CARGO_MANIFEST_DIR，输出 = src/generated。
pub fn generate_default() -> Result<(), String> {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").map_err(|e| e.to_string())?;
    let root = PathBuf::from(&manifest_dir);
    let out = root.join("src").join("generated");
    generate(&root, &out)
}
