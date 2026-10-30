//! 从能力包目录启动子进程 Guest（固定 `src/serve.mjs` / `src/serve.py` / 仓颉可执行产物）。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use ability_support::load_ability_package;
use ability_support::{
    Ability, AbilityManifest, AbilityProvider, Error, Result, RuntimeKind, guest_entry_path,
    GUEST_CJPM_TOML,
};
use async_trait::async_trait;

use crate::proc::ProcProvider;

/// 按能力根目录解析并拉起 Guest 子进程。
pub struct GuestSourceProvider {
    root: PathBuf,
    /// 仅认预构建产物：仓颉 Guest 不现场构建，缺产物直接报 `artifact_missing`。
    prebuilt_only: bool,
    /// 仓颉构建器程序名（默认 `cjpm`；测试可注入以模拟缺工具链）。
    cangjie_build_program: OsStringBox,
    cached: StdMutex<Option<AbilityManifest>>,
}

type OsStringBox = std::ffi::OsString;

impl GuestSourceProvider {
    pub fn from_root(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            prebuilt_only: false,
            cangjie_build_program: OsStringBox::from("cjpm"),
            cached: StdMutex::new(None),
        }
    }

    pub fn from_workspace_rel(workspace: &Path, rel: impl AsRef<Path>) -> Self {
        Self::from_root(workspace.join(rel.as_ref()))
    }

    /// 仅用预构建产物：不现场编译；缺产物归 `artifact_missing` 错误类。
    pub fn prebuilt_only(mut self, on: bool) -> Self {
        self.prebuilt_only = on;
        self
    }

    /// 覆盖仓颉构建器程序（默认 `cjpm`）。
    pub fn cangjie_build_program(mut self, program: impl Into<OsStringBox>) -> Self {
        self.cangjie_build_program = program.into();
        self
    }

    pub fn into_provider(self) -> Arc<dyn AbilityProvider> {
        Arc::new(self)
    }

    fn manifest_cached(&self) -> Result<AbilityManifest> {
        if let Some(m) = self.cached.lock().expect("cached").clone() {
            return Ok(m);
        }
        let meta = load_ability_package(&self.root).map_err(|e| Error::failed(e))?;
        let manifest = meta.to_manifest().map_err(|e| Error::failed(e))?;
        *self.cached.lock().expect("cached") = Some(manifest.clone());
        Ok(manifest)
    }

    fn proc_for_create(&self) -> Result<ProcProvider> {
        let meta = load_ability_package(&self.root).map_err(|e| Error::failed(e))?;
        match meta.runtime_kind {
            RuntimeKind::Rust => {
                return Err(Error::failed("GuestSourceProvider 不支持 Rust 进程内能力"));
            }
            RuntimeKind::Node => {
                let script = guest_entry_path(&self.root, RuntimeKind::Node)
                    .map_err(|e| Error::failed(e))?;
                Ok(ProcProvider::new("node")
                    .arg(script)
                    .current_dir(&self.root))
            }
            RuntimeKind::Python => {
                let script = guest_entry_path(&self.root, RuntimeKind::Python)
                    .map_err(|e| Error::failed(e))?;
                let src = self
                    .root
                    .join("src")
                    .canonicalize()
                    .map_err(|e| Error::io(e.to_string()))?;
                let sdk_src = self
                    .root
                    .ancestors()
                    .map(|anc| anc.join("SDK").join("ability-sdk-py").join("src"))
                    .find(|p| p.is_dir())
                    .and_then(|p| p.canonicalize().ok());
                let mut path = src.to_string_lossy().to_string();
                if let Some(sdk) = sdk_src {
                    let sep = if cfg!(windows) { ';' } else { ':' };
                    path = format!("{}{}{}", path, sep, sdk.to_string_lossy());
                }
                Ok(ProcProvider::new("python")
                    .arg("-u")
                    .arg(script)
                    .env("PYTHONPATH", path)
                    .current_dir(&self.root))
            }
            RuntimeKind::Cangjie => {
                let binary = self.cangjie_binary()?;
                Ok(ProcProvider::new(binary).current_dir(&self.root))
            }
        }
    }

    /// 定位仓颉 Guest 可执行产物；缺产物时按模式构建或报 `artifact_missing`。
    fn cangjie_binary(&self) -> Result<PathBuf> {
        let name = cangjie_package_name(&self.root)
            .ok_or_else(|| Error::failed(format!("无法从 {GUEST_CJPM_TOML} 解析包名")))?;
        if let Some(binary) = find_cangjie_binary(&self.root, &name) {
            return Ok(binary);
        }
        let primary = cangjie_binary_path(&self.root, &name);
        if self.prebuilt_only {
            return Err(Error::artifact_missing(format!(
                "仅用预构建模式：缺少仓颉产物 {}（请先构建后再装载）",
                primary.display()
            )));
        }
        self.cangjie_build(&primary)?;
        if let Some(binary) = find_cangjie_binary(&self.root, &name) {
            return Ok(binary);
        }
        Err(Error::artifact_missing(format!(
            "构建后仍未找到仓颉产物 {}（请检查 {GUEST_CJPM_TOML} output-type 与包名）",
            primary.display()
        )))
    }

    fn cangjie_build(&self, binary: &Path) -> Result<()> {
        use std::process::Command;
        let output = Command::new(&self.cangjie_build_program)
            .arg("build")
            .current_dir(&self.root)
            .output()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Error::toolchain_missing(format!(
                        "仓颉构建器 {} 不可用（未安装或不在 PATH）：{e}",
                        self.cangjie_build_program.to_string_lossy()
                    ))
                } else {
                    Error::io(e.to_string())
                }
            })?;
        if !output.status.success() {
            let tail = String::from_utf8_lossy(&output.stderr);
            let tail = tail.lines().rev().take(10).collect::<Vec<_>>();
            let tail: Vec<&str> = tail.into_iter().rev().collect();
            return Err(Error::failed(format!(
                "仓颉构建失败（期望产物 {}）：{}",
                binary.display(),
                tail.join("\n")
            )));
        }
        Ok(())
    }
}

/// 从 `cjpm.toml` 的 `[package] name = "..."` 解析包名。
pub fn cangjie_package_name(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join(GUEST_CJPM_TOML)).ok()?;
    let mut in_package = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(rest) = line.strip_prefix("name") {
            let rest = rest.trim_start();
            if let Some(rest) = rest.strip_prefix('=') {
                let v = rest.trim().trim_matches('"');
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// 仓颉 Guest 可执行产物约定路径：`target/release/<包名>[.exe]`。
pub fn cangjie_binary_path(root: &Path, package_name: &str) -> PathBuf {
    let mut p = root.join("target").join("release").join(package_name);
    if cfg!(windows) {
        p.set_extension("exe");
    }
    p
}

/// 在两种 cjpm 产物布局中定位可执行文件：
/// 约定布局 `target/release/<包名>[.exe]` 优先，其次 cjpm 1.x 的 `target/release/bin/main[.exe]`。
pub fn find_cangjie_binary(root: &Path, package_name: &str) -> Option<PathBuf> {
    let mut candidates = vec![cangjie_binary_path(root, package_name)];
    let mut main = root.join("target").join("release").join("bin").join("main");
    if cfg!(windows) {
        main.set_extension("exe");
    }
    candidates.push(main);
    candidates.into_iter().find(|p| p.is_file())
}

#[async_trait]
impl AbilityProvider for GuestSourceProvider {
    fn manifest(&self) -> AbilityManifest {
        self.manifest_cached()
            .unwrap_or_else(|_| crate::proc::unbound_guest_manifest())
    }

    async fn create(&self, config: serde_json::Value) -> Result<Arc<dyn Ability>> {
        let proc = self.proc_for_create()?;
        proc.create(config).await
    }
}
