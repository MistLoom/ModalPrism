//! 治理清单契约（采集进程侧）。
//!
//! 同一份方法集必须在四处一致：`surface` 声明、`ability.yml`、
//! 已装载 provider 的 manifest、`src/logic/*.rs` 文件集。
//! 任何一处漂移都让这条测试失败，避免能力面悄悄长出没人声明的方法。
//! 另有一条守住前端调用面：`api.ts` 里出现的每一个运维调用都必须被界面面放行，
//! 否则按钮在运行时必然被自家白名单拦死。

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
    #[serde(default)]
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
        // 协议声明：yml 与 provider manifest 必须一致（协议投影的唯一来源）。
        let decl = yml_decl(&dir);
        let declared_protocols: BTreeSet<String> = decl.protocols.iter().cloned().collect();
        let manifest_protocols: BTreeSet<String> = running
            .host()
            .manifest_protocols(ability.id)
            .into_iter()
            .collect();
        assert_eq!(
            declared_protocols, manifest_protocols,
            "{} 的 protocols 在 ability.yml 与 provider 声明之间漂移",
            ability.id
        );
    }

    // 运维面走 HTTP，被开放的方法必须声明 http 协议；协议声明不再与运行现实脱钩。
    for exposure in COLLECTOR_OPS {
        let protocols: BTreeSet<String> = yml_decl(&ability_dir(exposure.id))
            .protocols
            .into_iter()
            .collect();
        assert!(
            protocols.contains("http"),
            "运维面引用的 {} 没有在 ability.yml 声明 http 协议",
            exposure.id
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

/// 从源码里抽取 `call("<instance>", "<method>"` 形态的运维调用；
/// 只认字符串字面量，变量拼接的调用不在守护范围内。
fn frontend_calls(source: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let bytes = source.as_bytes();
    let mut offset = 0;
    while let Some(at) = source[offset..].find("call") {
        let start = offset + at;
        offset = start + 4;
        // `app.call(...)` 之类的方法调用不是运维入口。
        if start > 0 && bytes[start - 1] == b'.' {
            continue;
        }
        let mut rest = source[start + 4..].trim_start();
        if rest.starts_with('<') {
            let Some(end) = rest.find('>') else { continue };
            rest = rest[end + 1..].trim_start();
        }
        let Some(rest) = rest.strip_prefix('(') else {
            continue;
        };
        let Some(pair) = two_strings(rest) else {
            continue;
        };
        found.push(pair);
    }
    found
}

fn two_strings(text: &str) -> Option<(String, String)> {
    let (first, rest) = quoted(text)?;
    let rest = rest.trim_start().strip_prefix(',')?;
    let (second, _) = quoted(rest.trim_start())?;
    Some((first, second))
}

fn quoted(text: &str) -> Option<(String, &str)> {
    let rest = text.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some((rest[..end].to_string(), &rest[end + 1..]))
}

#[test]
fn frontend_only_calls_whitelisted_methods() {
    let api =
        fs::read_to_string(root().join("apps/modalprism-console/web/src/lib/api.ts")).unwrap();
    let mut checked = 0;
    for (instance, method) in frontend_calls(&api) {
        assert!(
            surface::exposes(CONSOLE, &instance, &method),
            "前端调用 {instance}.{method} 没有登记到界面面，运行时会被拦拒"
        );
        checked += 1;
    }
    assert!(
        checked >= 20,
        "扫描到的运维调用过少，解析可能失效：{checked}"
    );
}
