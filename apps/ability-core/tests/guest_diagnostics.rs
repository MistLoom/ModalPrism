//! Guest 装载失败分类与就绪探针（P0：装载成功 ≠ 可 invoke）。

use std::fs;
use std::path::PathBuf;

use ability_core::{AbilityHost, GuestSourceProvider, Readiness, cangjie_binary_path};
use ability_support::{ErrorKind, InstanceId, LoadSpec, SourceId};

fn sid(s: &str) -> SourceId {
    SourceId::new(s).unwrap()
}

fn iid(s: &str) -> InstanceId {
    InstanceId::new(s).unwrap()
}

/// 在系统临时目录组装一个最小仓颉 Guest 包（cjpm.toml + ability.yml + 入口）。
struct TempGuest {
    root: PathBuf,
}

impl TempGuest {
    fn cangjie(tag: &str, binary: Option<&[u8]>) -> Self {
        let root = std::env::temp_dir().join(format!(
            "leaf-guest-diag-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("cjpm.toml"),
            "[package]\n  name = \"diag_guest\"\n  output-type = \"executable\"\n",
        )
        .unwrap();
        fs::write(root.join("src/serve.cj"), "main() {}\n").unwrap();
        fs::write(
            root.join("ability.yml"),
            "framework_version: \"cangjie@1\"\nid: diag\nprotocols: [fun]\ndepends: []\nstateful: false\nmethods:\n  - id: ping\n    description: ping\n    input:\n      type: object\n",
        )
        .unwrap();
        if let Some(bytes) = binary {
            let path = cangjie_binary_path(&root, "diag_guest");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        Self { root }
    }

    fn provider(&self) -> GuestSourceProvider {
        GuestSourceProvider::from_root(&self.root)
    }
}

impl Drop for TempGuest {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn error_kind_codes_are_classified() {
    assert_eq!(ErrorKind::ToolchainMissing.code(), "toolchain_missing");
    assert_eq!(ErrorKind::ArtifactMissing.code(), "artifact_missing");
    assert_eq!(ErrorKind::HandshakeFailed.code(), "handshake_failed");
    // 往返：码可还原类别。
    for k in [
        ErrorKind::ToolchainMissing,
        ErrorKind::ArtifactMissing,
        ErrorKind::HandshakeFailed,
    ] {
        assert_eq!(ErrorKind::from_code(k.code()), Some(k));
    }
}

#[test]
fn cangjie_package_name_and_binary_path() {
    let g = TempGuest::cangjie("parse", None);
    assert_eq!(
        ability_core::cangjie_package_name(&g.root).as_deref(),
        Some("diag_guest")
    );
    let p = cangjie_binary_path(&g.root, "diag_guest");
    assert!(p.starts_with(g.root.join("target").join("release")));
}

#[tokio::test]
async fn prebuilt_only_missing_artifact_is_classified() {
    let host = AbilityHost::new();
    let g = TempGuest::cangjie("prebuilt-missing", None);
    host.install(
        sid("diag.prebuilt"),
        g.provider().prebuilt_only(true).into_provider(),
    )
    .await;
    let err = host
        .load(LoadSpec::new(sid("diag.prebuilt"), iid("diag.prebuilt")))
        .await
        .expect_err("缺预构建产物应装载失败");
    assert_eq!(err.code(), "artifact_missing");

    // 失败对调用方可见：就绪探针归 Failed，不在册。
    assert_eq!(
        host.readiness(&iid("diag.prebuilt")),
        Readiness::Failed {
            code: "artifact_missing",
            message: err.message().to_string(),
        }
    );
    assert_eq!(
        host.load_failures()
            .iter()
            .map(|f| f.code)
            .collect::<Vec<_>>(),
        vec!["artifact_missing"]
    );
    // 未装载实例探针为 Absent。
    assert_eq!(host.readiness(&iid("diag.absent")), Readiness::Absent);
}

#[tokio::test]
async fn missing_toolchain_is_classified() {
    // 注入不存在的构建器程序，模拟构建器不在 PATH。
    let host = AbilityHost::new();
    let g = TempGuest::cangjie("toolchain", None);
    host.install(
        sid("diag.toolchain"),
        g.provider()
            .cangjie_build_program("leaf-cjpm-missing-fixture")
            .into_provider(),
    )
    .await;
    let err = host
        .load(LoadSpec::new(sid("diag.toolchain"), iid("diag.toolchain")))
        .await
        .expect_err("缺构建器应装载失败");
    assert_eq!(err.code(), "toolchain_missing");
    assert!(err.message().contains("leaf-cjpm-missing-fixture"));
}

#[tokio::test]
async fn cangjie_source_mode_uses_prebuilt_binary() {
    // 无构建器环境：预置产物时不应触发构建，直接以产物为 Guest 程序。
    // 产物内容非合法 Guest → 装载必失败（spawn 失败归 io/toolchain，能运行则归 handshake），
    // 关键断言：绝不静默成功，且就绪探针不报 Ready。
    let host = AbilityHost::new();
    let g = TempGuest::cangjie("prebuilt-present", Some(b"not-a-guest\n"));
    host.install(sid("diag.ready"), g.provider().into_provider())
        .await;
    let result = host
        .load(LoadSpec::new(sid("diag.ready"), iid("diag.ready")))
        .await;
    if let Ok(_) = result {
        panic!("伪产物装载不应成功");
    }
    assert!(!matches!(host.readiness(&iid("diag.ready")), Readiness::Ready));
    assert!(matches!(
        host.load_failures().first().map(|f| f.code),
        Some("io") | Some("toolchain_missing") | Some("handshake_failed")
    ));
}

#[tokio::test]
async fn handshake_failure_is_classified() {
    // 进程能拉起但不讲 Guest 协议（立即退出）→ 归 handshake_failed。
    let ok = std::process::Command::new("node")
        .arg("--version")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        eprintln!("跳过：本机无 Node.js");
        return;
    }
    let host = AbilityHost::new();
    host.install(
        sid("diag.handshake"),
        std::sync::Arc::new(
            ability_core::ProcProvider::new("node")
                .arg("-e")
                .arg("process.exit(0)"),
        ),
    )
    .await;
    let err = host
        .load(LoadSpec::new(sid("diag.handshake"), iid("diag.handshake")))
        .await
        .expect_err("不讲协议的 Guest 应握手失败");
    assert_eq!(err.code(), "handshake_failed");
    assert!(matches!(
        host.readiness(&iid("diag.handshake")),
        Readiness::Failed { .. }
    ));
}

#[tokio::test]
async fn readiness_reflects_loaded_and_unloaded() {
    use ability_support::{
        Ability, AbilityCtx, AbilityId, AbilityManifest, FnAbility, MethodId, MethodMeta, Result,
    };
    use async_trait::async_trait;
    use serde_json::{Value, json};
    use std::sync::Arc;

    struct OkProvider;
    #[async_trait]
    impl ability_support::AbilityProvider for OkProvider {
        fn manifest(&self) -> AbilityManifest {
            AbilityManifest {
                id: AbilityId::new("ok.diag").unwrap(),
                depends: vec![],
                stateful: false,
                config_schema: Value::Null,
                methods: vec![MethodMeta {
                    id: MethodId::new("ping").unwrap(),
                    description: String::new(),
                    input_schema: Value::Null,
                    output_schema: None,
                }],
                events: vec![],
            }
        }
        async fn create(&self, _config: Value) -> Result<Arc<dyn Ability>> {
            Ok(Arc::new(FnAbility::new(
                self.manifest(),
                |_args: Value| async { Ok(json!(null)) },
            )))
        }
    }

    let host = AbilityHost::new();
    host.install(sid("ok.diag"), Arc::new(OkProvider)).await;
    host.load(LoadSpec::new(sid("ok.diag"), iid("ok.diag")))
        .await
        .unwrap();
    assert_eq!(host.readiness(&iid("ok.diag")), Readiness::Ready);
    assert!(host.load_failures().is_empty());

    host.unload(&iid("ok.diag")).await.unwrap();
    assert_eq!(host.readiness(&iid("ok.diag")), Readiness::Absent);
}

#[cfg(unix)]
#[tokio::test]
async fn build_failure_reports_output() {
    // 构建器存在但构建失败（`false` 立即非零退出）：归 failed，非静默成功。
    let host = AbilityHost::new();
    let g = TempGuest::cangjie("build-fail", None);
    host.install(
        sid("diag.buildfail"),
        g.provider().cangjie_build_program("false").into_provider(),
    )
    .await;
    let err = host
        .load(LoadSpec::new(sid("diag.buildfail"), iid("diag.buildfail")))
        .await
        .expect_err("构建失败应装载失败");
    assert_eq!(err.code(), "failed");
    assert!(err.message().contains("构建失败"));
}
