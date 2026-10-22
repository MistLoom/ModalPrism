//! 从能力包目录启动子进程 Guest（固定 `src/serve.mjs` / `src/serve.py`）。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use ability_support::load_ability_package;
use ability_support::{
    Ability, AbilityManifest, AbilityProvider, Error, Result, RuntimeKind, guest_entry_path,
};
use async_trait::async_trait;

use crate::proc::ProcProvider;

/// 按能力根目录解析并拉起 Guest 子进程。
pub struct GuestSourceProvider {
    root: PathBuf,
    cached: StdMutex<Option<AbilityManifest>>,
}

impl GuestSourceProvider {
    pub fn from_root(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            cached: StdMutex::new(None),
        }
    }

    pub fn from_workspace_rel(workspace: &Path, rel: impl AsRef<Path>) -> Self {
        Self::from_root(workspace.join(rel.as_ref()))
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
        }
    }
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
