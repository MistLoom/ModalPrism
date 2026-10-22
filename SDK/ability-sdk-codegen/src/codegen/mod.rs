//! 代码生成实现。

mod bridge;
mod cli;
mod handlers;
mod logic;
mod manifest;
mod provider;
mod write;

use std::path::Path;

use ability_support::AbilityPackageMeta;

pub fn write_all(meta: &AbilityPackageMeta, out_dir: &Path) -> Result<(), String> {
    let root = &meta.root;

    write::write_generated(
        &out_dir.join("manifest.rs"),
        &manifest::generate(meta)?,
    )?;

    if meta.decl.uses_handlers() {
        write::write_generated(
            &out_dir.join("handlers_trait.rs"),
            &handlers::generate(meta)?,
        )?;
        write::write_generated(&out_dir.join("bridge.rs"), &bridge::generate(meta)?)?;
        write::write_generated(
            &out_dir.join("provider.rs"),
            &provider::generate(meta)?,
        )?;

        let mod_rs = r#"// 自动生成，勿手改
pub mod manifest;
pub mod handlers_trait;
pub mod bridge;
pub mod provider;
"#;
        write::write_generated(&out_dir.join("mod.rs"), mod_rs)?;

        logic::scaffold_logic(root, meta)?;
    } else {
        write::write_generated(
            &out_dir.join("provider.rs"),
            &provider::generate_manifest_only(meta)?,
        )?;
        write::write_generated(
            &out_dir.join("mod.rs"),
            r#"// 自动生成，勿手改
pub mod manifest;
pub mod provider;
"#,
        )?;
    }

    let lib_rs = root.join("src").join("lib.rs");
    let lib_stub = if meta.decl.uses_handlers() {
        r#"mod generated;
mod logic;

pub use generated::provider::provider;
"#
    } else {
        r#"mod generated;

pub use generated::manifest::{manifest, ABILITY_ID};
"#
    };
    write::write_scaffold_once(&lib_rs, lib_stub)?;

    if let Some(cli) = &meta.cli {
        let bin_dir = root.join("src").join("bin");
        let bin_path = bin_dir.join(format!("{}.rs", cli.binary));
        let bin_rs = cli::generate_bin(meta, cli)?;
        write::write_scaffold_once(&bin_path, &bin_rs)?;
    }

    Ok(())
}

#[derive(Clone, Debug, Default)]
pub struct CodegenOptions {
    pub emit_cli_bin: bool,
}

pub use write::{ensure_mod_declarations, write_generated, write_scaffold_once};
