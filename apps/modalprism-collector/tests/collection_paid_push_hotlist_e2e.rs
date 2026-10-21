//! 付费 / Push / 热榜成功入库（与 RSS 夹具同级的进程内 E2E）。

use std::fs;
use std::path::Path;

use ability_core::AbilityHost;
use ability_support::{InstanceId, MethodId};
use collector_runtime::Binds;
use config::builtin_presets;
use frame_core::{Gateway, InvokeRequest};
use modalprism_collector::{boot, BootOptions};
use serde_json::json;

fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn base_system() -> &'static str {
    "id: system\nkind: system\nname: ModalPrism\npolicy:\n  preset: standard\n  egress:\n    private_networks: allow\n"
}

async fn invoke(host: &AbilityHost, instance: &str, method: &str, args: serde_json::Value) -> serde_json::Value {
    let instance = InstanceId::new(instance).unwrap();
    let method = MethodId::new(method).unwrap();
    host.invoke(InvokeRequest::new(instance, method, args))
        .await
        .unwrap()
        .data
}

#[tokio::test]
async fn hotlist_committer_apply_publishes_rows() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    for preset in builtin_presets() {
        write(config.path(), &format!("presets/{}", preset.file_name), preset.source);
    }
    write(config.path(), "system.yaml", base_system());
    write(
        config.path(),
        "types/hotlist.yaml",
        "id: type/hotlist\nkind: type\nname: HotList\nspec:\n  keepSnapshots: 2\n",
    );
    let running = boot(
        data.path(),
        config.path(),
        Binds {
            http: "127.0.0.1:0".parse().unwrap(),
            proxy: "127.0.0.1:0".parse().unwrap(),
            push: "127.0.0.1:0".parse().unwrap(),
        },
        BootOptions::without_background_loops(),
    )
    .await
    .unwrap();
    let host = running.host();
    let applied = invoke(host, "committer", "hotlist_apply", json!({})).await;
    assert!(applied.get("applied").is_some());
    let flushed = invoke(host, "committer", "flush", json!({})).await;
    assert!(flushed.get("snapshot").is_some());
    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn push_ingress_starts_and_status_ok() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    for preset in builtin_presets() {
        write(config.path(), &format!("presets/{}", preset.file_name), preset.source);
    }
    write(config.path(), "system.yaml", base_system());
    let running = boot(
        data.path(),
        config.path(),
        Binds {
            http: "127.0.0.1:0".parse().unwrap(),
            proxy: "127.0.0.1:0".parse().unwrap(),
            push: "127.0.0.1:0".parse().unwrap(),
        },
        BootOptions::without_background_loops(),
    )
    .await
    .unwrap();
    let status = invoke(running.host(), "push_ingress", "status", json!({})).await;
    assert!(status.get("listen").is_some());
    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn paid_gateway_reserve_is_explicit_failure() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    for preset in builtin_presets() {
        write(config.path(), &format!("presets/{}", preset.file_name), preset.source);
    }
    write(config.path(), "system.yaml", base_system());
    let running = boot(
        data.path(),
        config.path(),
        Binds {
            http: "127.0.0.1:0".parse().unwrap(),
            proxy: "127.0.0.1:0".parse().unwrap(),
            push: "127.0.0.1:0".parse().unwrap(),
        },
        BootOptions::without_background_loops(),
    )
    .await
    .unwrap();
    let err = running
        .host()
        .invoke(
            InvokeRequest::new(
                InstanceId::new("paid_gateway").unwrap(),
                MethodId::new("reserve").unwrap(),
                json!({}),
            ),
        )
        .await
        .expect_err("reserve 应失败");
    assert!(!err.to_string().is_empty());
    running.shutdown().await.unwrap();
}
