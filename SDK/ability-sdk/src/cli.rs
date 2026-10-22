//! SDK CLI 编译辅助。

use std::path::Path;
use std::process::Command;

/// 在 workspace 根目录编译指定能力的 CLI 二进制。
pub fn build_standalone(
    workspace_root: impl AsRef<Path>,
    package: &str,
    bin: &str,
    profile: &str,
) -> Result<(), String> {
    let status = Command::new("cargo")
        .current_dir(workspace_root.as_ref())
        .args(["build", "-p", package, "--bin", bin, "--profile", profile])
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("编译 {package}/{bin} 失败"))
    }
}
