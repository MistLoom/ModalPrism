//! 治理清单契约（只读进程侧）。
//!
//! `surface::AGENT` 声明的能力与方法，必须与已装载 provider 的 manifest、
//! `ability.yml`、`src/logic/*.rs` 文件集完全一致。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use surface::{AGENT, Ability};

fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn config_dir(dir: &Path) {
    for preset in config::builtin_presets() {
        write(dir, &format!("presets/{}", preset.file_name), preset.source);
    }
    write(
        dir,
        "system.yaml",
        "id: system\nkind: system\nname: ModalPrism\npolicy:\n  preset: standard\nstrata:\n  gateway:\n    blob_backend: dir\n",
    );
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn ability_dir(id: &str) -> PathBuf {
    let root = root();
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
    methods: Vec<Declared>,
}

#[derive(serde::Deserialize)]
struct Declared {
    id: String,
}

fn yml_methods(dir: &Path) -> BTreeSet<String> {
    let source = fs::read_to_string(dir.join("ability.yml")).unwrap();
    let decl: Decl = serde_yaml::from_str(&source).unwrap();
    decl.methods.into_iter().map(|item| item.id).collect()
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
async fn agent_surface_matches_providers_manifests_and_logic() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let running = modalprism_agent::boot(data.path(), config.path())
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
    for ability in AGENT {
        declared.insert(ability.id.to_string(), declared_methods(ability));
    }

    assert_eq!(
        declared, manifest,
        "surface::AGENT 与已装载 provider 的 manifest 不一致"
    );

    for ability in AGENT {
        // web 是进程内合成投影（代理回环领域 API），无 ability.yml 与 logic 目录。
        let candidate = root().join("abilitys").join(ability.id);
        if !candidate.join("ability.yml").is_file() {
            continue;
        }
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
    }
}
