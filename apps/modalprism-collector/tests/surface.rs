//! 治理清单契约：surface 声明、ability.yml、provider manifest 与 logic 文件集一致。
//!
//! 公共 HTTP 已由 `domain-api` 承担，前端不再直接调用 Ability；这里只守住
//! collector 的安装面、运维面与只读 Agent 面。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use assembly::Binds;
use config::builtin_presets;
use surface::{AGENT, AGENT_MCP, Ability, COLLECTOR, COLLECTOR_OPS};

fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn config_dir(dir: &Path) {
    for preset in builtin_presets() {
        write(dir, &format!("presets/{}", preset.file_name), preset.source);
    }
    write(
        dir,
        "system.yaml",
        "id: system\nkind: system\nname: ModalPrism\npolicy:\n  preset: standard\nstrata:\n  gateway:\n    blob_backend: dir\n",
    );
}

fn binds() -> Binds {
    Binds {
        http: "127.0.0.1:0".parse().unwrap(),
        proxy: "127.0.0.1:0".parse().unwrap(),
        push: "127.0.0.1:0".parse().unwrap(),
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn ability_dir(id: &str) -> PathBuf {
    let root = root();
    let local = root.join("apps/modalprism-collector/ability").join(id);
    if local.join("ability.yml").is_file() {
        return local;
    }
    for category in fs::read_dir(root.join("abilitys")).unwrap() {
        let candidate = category.unwrap().path().join(id);
        if candidate.join("ability.yml").is_file() {
            return candidate;
        }
    }
    panic!("找不到能力 {id} 的 ability.yml");
}

#[derive(serde::Deserialize)]
struct Decl {
    protocols: Vec<String>,
    methods: Vec<Declared>,
}

#[derive(serde::Deserialize)]
struct Declared {
    id: String,
}

fn yml_decl(dir: &Path) -> Decl {
    let source = fs::read_to_string(dir.join("ability.yml")).unwrap();
    serde_yaml::from_str(&source).unwrap()
}

fn yml_methods(dir: &Path) -> BTreeSet<String> {
    yml_decl(dir)
        .methods
        .into_iter()
        .map(|item| item.id)
        .collect()
}

fn logic_methods(dir: &Path) -> BTreeSet<String> {
    fs::read_dir(dir.join("src/logic"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .filter_map(|path| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .filter(|stem| stem != "mod")
        .collect()
}

fn declared_methods(ability: &Ability) -> BTreeSet<String> {
    ability
        .methods
        .iter()
        .map(|item| item.to_string())
        .collect()
}

#[tokio::test]
async fn collector_surface_matches_providers_manifests_and_logic() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let running = modalprism_collector::boot(data.path(), config.path(), binds())
        .await
        .unwrap();

    let mut manifest: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for entry in running.host().catalog() {
        manifest
            .entry(entry.instance.as_str().to_string())
            .or_default()
            .insert(entry.method.as_str().to_string());
    }
    let mut declared: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for ability in COLLECTOR {
        declared.insert(ability.id.to_string(), declared_methods(ability));
    }
    assert_eq!(
        declared, manifest,
        "surface::COLLECTOR 与已装载 provider 的 manifest 不一致"
    );

    for ability in COLLECTOR {
        let dir = ability_dir(ability.id);
        let expected = declared_methods(ability);
        assert_eq!(
            expected,
            yml_methods(&dir),
            "{} 的 ability.yml 漂移",
            ability.id
        );
        assert_eq!(
            expected,
            logic_methods(&dir),
            "{} 的 logic 文件漂移",
            ability.id
        );
        let decl = yml_decl(&dir);
        let declared_protocols: BTreeSet<String> = decl.protocols.into_iter().collect();
        // 协议不再进 manifest：以 host 实际登记的 protocol_support 为准。
        let manifest_protocols: BTreeSet<String> = running
            .host()
            .protocol_support(ability.id)
            .into_iter()
            .map(|kind| kind.as_str().to_string())
            .collect();
        assert_eq!(
            declared_protocols, manifest_protocols,
            "{} 的 protocols 在 ability.yml 与 provider 声明之间漂移",
            ability.id
        );
    }

    for exposure in COLLECTOR_OPS {
        let protocols: BTreeSet<String> = yml_decl(&ability_dir(exposure.id))
            .protocols
            .into_iter()
            .collect();
        assert!(
            protocols.contains("http"),
            "运维面引用的 {} 没有声明 http 协议",
            exposure.id
        );
        let ability = surface::ability(COLLECTOR, exposure.id).expect("运维面引用了未安装的能力");
        for method in exposure.methods {
            assert!(
                ability.has(method),
                "运维面 {} 未声明 {method}",
                exposure.id
            );
        }
    }

    for exposure in AGENT_MCP {
        let ability = surface::ability(AGENT, exposure.id).expect("MCP 面引用了未安装的能力");
        for method in exposure.methods {
            assert!(
                ability.has(method),
                "MCP 面 {} 未声明 {method}",
                exposure.id
            );
            assert!(
                ability.is_read_only(method) || surface::mcp_write_allowed(exposure.id, method),
                "MCP 面 {} {method} 既不是只读方法，也不在写白名单",
                exposure.id
            );
        }
    }

    // 写白名单之外：破坏性删除、系统设置与采集驱动写永远不得出现在任何 MCP 面。
    for (id, method) in [
        ("catalog_admin", "remove"),
        ("catalog_admin", "proxy"),
        ("workflow", "save"),
        ("workflow", "save_model"),
        ("workflow", "remove"),
        ("workflow", "remove_model"),
        ("workflow", "tick"),
        ("workflow", "toggle"),
    ] {
        assert!(
            !surface::exposes(AGENT_MCP, id, method) && !surface::mcp_write_allowed(id, method),
            "MCP 面/写白名单不得放开 {id}.{method}"
        );
    }

    running.shutdown().await.unwrap();
}
