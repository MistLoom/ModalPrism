//! 治理清单契约（采集进程侧）。
//!
//! 同一份方法集必须在四处一致：`surface` 声明、`ability.yml`、
//! 已装载 provider 的 manifest、`src/logic/*.rs` 文件集。
//! 任何一处漂移都让这条测试失败，避免能力面悄悄长出没人声明的方法。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use assembly::Binds;
use config::builtin_presets;
use surface::{Ability, COLLECTOR, COLLECTOR_OPS, CONSOLE};

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

/// 能力目录：应用自有能力在 `ability/` 下，共享能力在 `abilitys/<科目>/` 下。
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
async fn collector_surface_matches_providers_manifests_and_logic() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let running = modalprism_collector::boot(data.path(), config.path(), binds())
        .await
        .unwrap();

    // manifest：已装载 provider 声明的方法集。
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
    }

    // 开放面只能收窄，不能凭空多出方法。
    for exposure in COLLECTOR_OPS {
        let ability = surface::ability(COLLECTOR, exposure.id).expect("运维面引用了未安装的能力");
        for method in exposure.methods {
            assert!(
                ability.has(method),
                "运维面 {} 未声明 {method}",
                exposure.id
            );
        }
    }
    for exposure in CONSOLE {
        let ability =
            surface::ability(COLLECTOR, exposure.id).expect("界面面引用了采集进程未安装的能力");
        for method in exposure.methods {
            assert!(
                ability.has(method),
                "界面面 {} 未声明 {method}",
                exposure.id
            );
            assert!(
                surface::exposes(COLLECTOR_OPS, exposure.id, method),
                "界面面 {} {method} 超出运维面",
                exposure.id
            );
        }
    }

    running.shutdown().await.unwrap();
}
