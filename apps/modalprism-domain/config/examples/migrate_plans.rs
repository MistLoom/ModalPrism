//! 一次性迁移脚本：把信源里的 `schedule` / `admission.initialBackfill` 迁出为
//! 一等采集计划（`config/plans/*.yaml`）。幂等：已迁移（无 schedule）的信源自动跳过。
//!
//! 用法：`cargo run -p config --example migrate_plans [配置目录]`
//! 缺省配置目录为仓库根 `config/`。迁移经整库 `validate` 原子兜底，任一失败全量回滚。

use std::path::{Path, PathBuf};

fn main() {
    let dir: PathBuf = match std::env::args().nth(1) {
        Some(arg) => PathBuf::from(arg),
        None => Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../config")
            .canonicalize()
            .expect("仓库 config 目录应存在"),
    };
    let options = config::LoadOptions::default();
    match config::migrate_schedules_to_plans(&dir, &options) {
        Ok(moved) => println!(
            "schedule→plan 迁移完成：生成 {moved} 份计划（目录 {}）",
            dir.display()
        ),
        Err(error) => {
            eprintln!("迁移失败（已回滚）: {error}");
            std::process::exit(1);
        }
    }
}
