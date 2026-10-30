//! 启动自愈：配置目录不可加载时用出厂快照替换，并完成已下线目录的退役清理。
//!
//! 一份畸形的声明会让整次校验失败，而进程只能以配置快照启动——直接失败退出等于把
//! 「一个坏文件」升级成「系统起不来」。这里按固定顺序降级：先正常加载；失败则用
//! 数据目录里的出厂快照替换（原配置完整备份），恢复也失败才把原始错误抛回去。
//!
//! 加载成功后还会做一次退役清理：加载器会跳过已下线目录，但「跳过」不等于「清理」——
//! 留在目录里就会跟着出厂基线一起被复制回去，恢复出厂永远清不干净。

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use config::{LoadOptions, Snapshot};

/// 加载配置；配置目录不可加载时尝试用出厂快照恢复。
///
/// 无论走哪条路径，成功后都会把已下线目录从 live 配置与出厂基线里移走：否则下次
/// 恢复出厂又会把历史垃圾复制回来。
pub(super) fn load_or_recover(
    data: &Path,
    config_dir: &Path,
    options: &LoadOptions,
) -> Result<Snapshot, config::Error> {
    let snapshot = load(data, config_dir, options)?;
    let factory = data.join("factory").join("config");
    let moved = retire(data, config_dir) + retire(data, &factory);
    if moved > 0 {
        tracing::warn!(moved, "已下线配置目录完成退役清理");
    }
    Ok(snapshot)
}

fn load(data: &Path, config_dir: &Path, options: &LoadOptions) -> Result<Snapshot, config::Error> {
    match config::validate_with_warnings(config_dir, options) {
        Ok((snapshot, warnings)) => {
            log_warnings(&warnings);
            Ok(snapshot)
        }
        Err(error) => {
            tracing::error!(detail = %error, "配置目录校验失败，尝试用出厂快照恢复");
            match restore(data, config_dir, options) {
                Ok(()) => match config::validate_with_warnings(config_dir, options) {
                    Ok((snapshot, warnings)) => {
                        log_warnings(&warnings);
                        tracing::warn!("已用出厂快照恢复配置目录，原配置已备份到数据目录的 backup 下");
                        Ok(snapshot)
                    }
                    Err(error) => {
                        tracing::error!(detail = %error, "恢复出厂快照后仍不可加载");
                        Err(error)
                    }
                },
                Err(reason) => {
                    tracing::error!(reason = %reason, "出厂快照恢复失败");
                    Err(error)
                }
            }
        }
    }
}

/// 用 `<data>/factory/config` 覆盖配置目录；失败时尽力还原原配置。
fn restore(data: &Path, config_dir: &Path, options: &LoadOptions) -> Result<(), String> {
    let factory = data.join("factory").join("config");
    if !factory.is_dir() {
        return Err("出厂快照不存在".into());
    }
    if let Err(error) = config::validate(&factory, options) {
        return Err(format!("出厂快照本身不可加载: {error}"));
    }
    let backup = data
        .join("backup")
        .join(format!("config-before-recover-{}", stamp()));
    // 配置目录被整个删掉时没有可备份的内容：直接落到出厂快照就是全部重建。
    if config_dir.exists()
        && let Err(error) = copy_tree(config_dir, &backup)
    {
        return Err(format!("备份当前配置失败: {error}"));
    }
    if let Err(error) = replace(config_dir, &factory) {
        rollback(config_dir, &backup);
        return Err(format!("写入出厂配置失败: {error}"));
    }
    if let Err(error) = config::validate(config_dir, options) {
        rollback(config_dir, &backup);
        return Err(format!("恢复后仍不可加载: {error}"));
    }
    Ok(())
}

/// 用备份覆盖回去；没有备份时保留出厂副本，至少目录是可加载的。
fn rollback(config_dir: &Path, backup: &Path) {
    if backup.is_dir() {
        let _ = replace(config_dir, backup);
    }
}

/// 把已下线的配置目录整体移到 `backup/` 下，返回移走的条目数。
///
/// 只动 [`config::retired_paths`] 登记的顶层路径；备份而不是直接删除，是因为这些文件
/// 可能还有人工审计价值，且移动失败不能阻断启动。
fn retire(data: &Path, root: &Path) -> usize {
    if !root.is_dir() {
        return 0;
    }
    let mut moved = 0;
    for rel in config::retired_paths() {
        let path = root.join(rel);
        if !path.exists() {
            continue;
        }
        let target = data
            .join("backup")
            .join(format!("retired-{}", stamp()))
            .join(rel);
        match move_path(&path, &target) {
            Ok(()) => {
                moved += 1;
                tracing::warn!(
                    path = %path.display(),
                    backup = %target.display(),
                    since = config::retired_since(rel).unwrap_or("已下线"),
                    "已下线配置目录已退役"
                );
            }
            Err(error) => {
                tracing::error!(path = %path.display(), detail = %error, "退役清理失败");
            }
        }
    }
    moved
}

/// 优先整目录重命名（同盘瞬间完成），跨盘时退回复制后删除。
fn move_path(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    if from.is_dir() {
        copy_tree(from, to)?;
        std::fs::remove_dir_all(from)
    } else {
        std::fs::copy(from, to)?;
        std::fs::remove_file(from)
    }
}

fn log_warnings(warnings: &[config::Diagnostic]) {
    for warning in warnings {
        tracing::warn!(
            file = %warning.file,
            line = warning.line,
            message = %warning.message,
            "配置项已跳过"
        );
    }
}

/// 清空目标目录后整棵复制过去。
fn replace(target: &Path, source: &Path) -> std::io::Result<()> {
    clear(target)?;
    copy_tree(source, target)
}

fn clear(dir: &Path) -> std::io::Result<()> {
    if !dir.exists() {
        return std::fs::create_dir_all(dir);
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            std::fs::remove_dir_all(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let source = entry.path();
        let target = to.join(entry.file_name());
        if source.is_dir() {
            copy_tree(&source, &target)?;
        } else {
            std::fs::copy(&source, &target)?;
        }
    }
    Ok(())
}

fn stamp() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|span| span.as_nanos())
        .unwrap_or_default();
    format!("{nanos}")
}
