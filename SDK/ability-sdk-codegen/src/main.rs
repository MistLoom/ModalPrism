//! CLI：`ability-sdk-codegen --target rust|ts|py [--root DIR] [--out DIR]`

use std::env;
use std::path::PathBuf;

use ability_sdk_codegen::{CodegenTarget, generate, write_target};

fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    let mut target = CodegenTarget::Rust;
    let mut root: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--target" | "-t" => {
                i += 1;
                target = CodegenTarget::parse(args.get(i).ok_or("--target 需要值")?)?;
            }
            "--root" | "-r" => {
                i += 1;
                root = Some(PathBuf::from(args.get(i).ok_or("--root 需要值")?));
            }
            "--out" | "-o" => {
                i += 1;
                out = Some(PathBuf::from(args.get(i).ok_or("--out 需要值")?));
            }
            "--help" | "-h" => {
                eprintln!(
                    "用法: ability-sdk-codegen [-t rust|ts|py] [-r ROOT] [-o OUT_DIR]\n默认: rust, root=CARGO_MANIFEST_DIR 或当前目录, out=src/generated|generated"
                );
                return Ok(());
            }
            other => return Err(format!("未知参数: {other}")),
        }
        i += 1;
    }
    let root = root.unwrap_or_else(|| {
        env::var("CARGO_MANIFEST_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| env::current_dir().unwrap_or_default())
    });
    let out = out.unwrap_or_else(|| {
        if matches!(target, CodegenTarget::Rust) {
            root.join("src").join("generated")
        } else {
            root.join("generated")
        }
    });
    match target {
        CodegenTarget::Rust => generate(&root, &out)?,
        CodegenTarget::Ts | CodegenTarget::Py => {
            let meta = ability_support::load_ability_package(&root)?;
            write_target(target, &meta, &out)?;
        }
    }
    Ok(())
}
