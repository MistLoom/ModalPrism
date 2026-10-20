//! 在编译采集进程时构建控制台前端（若本机有 Node 且产物缺失或过期）。

use std::path::Path;
use std::process::Command;

fn main() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let web = manifest.join("../console-web");
    let dist = web.join("dist/index.html");
    let src = web.join("src");
    println!("cargo:rerun-if-changed={}", src.display());
    println!("cargo:rerun-if-changed={}", web.join("index.html").display());
    println!("cargo:rerun-if-changed={}", web.join("package.json").display());
    if std::env::var("SKIP_CONSOLE_WEB_BUILD").is_ok() {
        return;
    }
    let needs_build = !dist.is_file() || newer_than(&dist, &src);
    if !needs_build {
        return;
    }
    if which_npm().is_none() {
        write_minimal_dist(&web.join("dist"));
        return;
    }
    let npm = which_npm().unwrap();
    let node_modules = web.join("node_modules");
    if !node_modules.is_dir() {
        let status = Command::new(&npm)
            .args(["ci", "--offline"])
            .current_dir(&web)
            .status();
        if status.is_err() || !status.unwrap().success() {
            let _ = Command::new(&npm)
                .args(["ci"])
                .current_dir(&web)
                .status();
        }
    }
    let status = Command::new(&npm)
        .args(["run", "build"])
        .current_dir(&web)
        .status()
        .expect("npm run build");
    if !status.success() || !dist.is_file() {
        write_minimal_dist(&web.join("dist"));
    }
}

fn write_minimal_dist(dist: &Path) {
    std::fs::create_dir_all(dist).ok();
    let html = r#"<!DOCTYPE html><html lang="zh-CN"><head><meta charset="utf-8"><title>ModalPrism</title></head>
<body><p>请执行 npm run build 后重新编译采集进程。</p></body></html>"#;
    std::fs::write(dist.join("index.html"), html).ok();
}

fn which_npm() -> Option<String> {
    Command::new("npm")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|_| "npm".into())
}

fn newer_than(dist: &Path, src: &Path) -> bool {
    let dist_mtime = dist.metadata().and_then(|m| m.modified()).ok();
    let src_mtime = walk_newest(src).or_else(|| src.metadata().and_then(|m| m.modified()).ok());
    match (dist_mtime, src_mtime) {
        (Some(d), Some(s)) => s > d,
        _ => true,
    }
}

fn walk_newest(path: &Path) -> Option<std::time::SystemTime> {
    let mut best = path.metadata().and_then(|m| m.modified()).ok();
    if path.is_dir() {
        for entry in std::fs::read_dir(path).into_iter().flatten().flatten() {
            if let Some(t) = walk_newest(&entry.path()) {
                if best.is_none_or(|b| t > b) {
                    best = Some(t);
                }
            }
        }
    }
    best
}
