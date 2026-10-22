//! 能力 Catalog：声明索引，非运行时白名单。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use ability_support::{ProtocolKind, discover_ability_roots, load_ability_package};
use frame_core::AccessKind;

/// 非运行时能力目录（有声明但不编入 Provider 注册表）。
const EXCLUDE_FOLDERS: &[&str] = &["devkit", "codegen_fixture", "test_feedback"];

/// workspace 根目录。
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

pub fn ability_path(rel_root: &str) -> PathBuf {
    workspace_root().join(rel_root)
}

/// 单条 source 声明索引。
#[derive(Clone, Debug)]
pub struct SourceEntry {
    pub id: String,
    /// 属主应用，取自 `apps/<app>/ability`。
    pub app: String,
    /// 其他应用可否调用。
    pub open: bool,
    pub rel_root: String,
    pub root: PathBuf,
    /// 当前二进制是否链入了 Provider。
    pub linked: bool,
}

/// 工作区能力 Catalog。
#[derive(Clone, Debug, Default)]
pub struct WorkspaceCatalog {
    sources: Vec<SourceEntry>,
}

impl WorkspaceCatalog {
    /// 扫描 `abilitys` 与 `apps/*/ability`，并标记 linked 状态。
    pub fn discover() -> Self {
        let ws = workspace_root();
        let linked: BTreeSet<String> = crate::registry::linked_ids()
            .into_iter()
            .map(String::from)
            .collect();
        let mut sources = Vec::new();

        for root in discover_ability_roots(&ws) {
            let rel = root
                .strip_prefix(&ws)
                .unwrap_or(&root)
                .to_string_lossy()
                .replace('\\', "/");
            if rel.contains("/templates/") {
                continue;
            }
            let folder = root.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if EXCLUDE_FOLDERS.contains(&folder) {
                continue;
            }
            let Ok(pkg) = load_ability_package(&root) else {
                continue;
            };
            let id = pkg.decl.id.clone();
            let app = owner_app(&rel);
            sources.push(SourceEntry {
                id: id.clone(),
                app,
                open: pkg.decl.open,
                rel_root: rel,
                root: root.clone(),
                linked: linked.contains(&id),
            });
        }

        sources.sort_by(|a, b| a.id.cmp(&b.id));
        Self { sources }
    }

    pub fn sources(&self) -> &[SourceEntry] {
        &self.sources
    }

    pub fn get(&self, id: &str) -> Option<&SourceEntry> {
        self.sources.iter().find(|s| s.id == id)
    }

    pub fn linked_sources(&self) -> impl Iterator<Item = &SourceEntry> {
        self.sources.iter().filter(|s| s.linked)
    }

    pub fn registered_count(&self) -> usize {
        self.sources.iter().filter(|s| s.linked).count()
    }

    /// 静态检索：声明支持指定协议的能力清单（含未 load 项）。
    pub fn sources_by_protocol(&self, kind: ProtocolKind) -> Vec<&SourceEntry> {
        self.sources
            .iter()
            .filter(|s| {
                load_ability_package(&s.root)
                    .map(|p| p.supports(kind))
                    .unwrap_or(false)
            })
            .collect()
    }
}

/// `abilitys/...` 归属 `common`（共享能力）；`apps/<app>/ability/...` 归属该应用。
pub fn owner_app(rel: &str) -> String {
    let mut parts = rel.split('/');
    match parts.next() {
        Some("abilitys") => "common".into(),
        Some("apps") => parts.next().unwrap_or("").into(),
        _ => String::new(),
    }
}

/// 解析能力支持的端口协议（来自 `ability.yml` → `protocols`）。
pub fn resolve_protocols(ability_root: &Path) -> Vec<AccessKind> {
    if ability_root.join("ability.yml").is_file() {
        if let Ok(meta) = load_ability_package(ability_root) {
            let mut kinds: Vec<AccessKind> = meta
                .protocols
                .iter()
                .filter_map(|p| AccessKind::from_protocol(*p))
                .collect();
            kinds.sort_by_key(|k| k.as_str());
            kinds.dedup();
            return kinds;
        }
    }
    vec![AccessKind::Http, AccessKind::Fun]
}

/// 注册实例协议支持到 Host。
pub fn register_protocol_support(
    host: &ability_core::AbilityHost,
    instance: &str,
    ability_root: &Path,
) {
    let kinds = resolve_protocols(ability_root);
    host.set_protocol_support(instance, kinds);
}
