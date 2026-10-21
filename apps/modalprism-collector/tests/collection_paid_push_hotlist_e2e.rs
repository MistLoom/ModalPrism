//! 付费 / Push / 热榜成功入库（进程内 E2E）。

use std::fs;
use std::path::Path;

use ability_core::AbilityHost;
use ability_support::{InstanceId, MethodId};
use collector_runtime::Binds;
use config::builtin_presets;
use frame_core::{Gateway, InvokeRequest};
use modalprism_collector::{boot, BootOptions};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

static BOOT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn base_system() -> &'static str {
    "id: system\nkind: system\nname: ModalPrism\npolicy:\n  preset: standard\n  egress:\n    private_networks: allow\n"
}

async fn invoke(host: &AbilityHost, instance: &str, method: &str, args: serde_json::Value) -> serde_json::Value {
    host.invoke(InvokeRequest::new(
        InstanceId::new(instance).unwrap(),
        MethodId::new(method).unwrap(),
        args,
    ))
    .await
    .unwrap()
    .data
}

async fn spawn_json_api(body: &str) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let body = body.to_string();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let body = body.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    port
}

fn scaffold_paid(config: &Path, endpoint: &str) {
    for preset in builtin_presets() {
        write(config, &format!("presets/{}", preset.file_name), preset.source);
    }
    write(config, "system.yaml", base_system());
    let config_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config");
    write(
        config,
        "accessors/paid-api.yaml",
        &fs::read_to_string(config_root.join("accessors/paid-api.yaml")).unwrap(),
    );
    write(
        config,
        "sources/fixture-paid.yaml",
        &format!(
            r#"id: source/fixture-paid
kind: source
name: Fixture Paid
access:
  accessor: accessor/paid-api
  locator:
    endpoint: {endpoint}
  spec:
    service: search
    method: GET
    params: {{}}
    pages: 1
paid:
  budget:
    requests: 48
    window: 1d
pipeline: []
schedule:
  interval: 1h
admission:
  initialBackfill:
    limit: 5
    maxAge: 365d
"#
        ),
    );
}

async fn run_planner_round(host: &AbilityHost, source: &str) {
    let triggered = invoke(
        host,
        "planner",
        "trigger",
        json!({ "source": source }),
    )
    .await;
    if let Some(runs) = triggered.get("runs").and_then(serde_json::Value::as_array) {
        assert!(!runs.is_empty(), "trigger 应执行采集: {triggered:?}");
        for run in runs {
            assert!(
                run.get("ok").and_then(serde_json::Value::as_bool) == Some(true),
                "run_task: {run:?}"
            );
        }
        return;
    }
    let tasks = if triggered
        .get("tasks")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|a| !a.is_empty())
    {
        triggered.get("tasks").cloned().unwrap()
    } else {
        invoke(host, "planner", "tick", json!({})).await
    };
    for task in tasks.as_array().expect("tasks") {
        let outcome = invoke(host, "runner", "run_task", json!({ "task": task.clone() })).await;
        assert!(
            outcome.get("ok").and_then(|v| v.as_bool()) == Some(true),
            "run_task: {outcome:?}"
        );
    }
}

#[tokio::test]
async fn paid_fixture_ingests_and_budget_tracks_receipt() {
    let _guard = BOOT_LOCK.lock().await;
    let api_body = r#"{"title":"Paid fixture","url":"https://fixture.local/paid-1"}"#;
    let port = spawn_json_api(api_body).await;
    let endpoint = format!("http://127.0.0.1:{port}/v1/search");
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    scaffold_paid(config.path(), &endpoint);

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

    run_planner_round(host, "fixture-paid").await;
    invoke(host, "committer", "flush", json!({})).await;

    let listed = invoke(
        host,
        "strata_read",
        "query",
        json!({
            "query": {
                "from": "resources",
                "limit": 20
            }
        }),
    )
    .await;
    let rows = listed.get("rows").and_then(|v| v.as_array()).unwrap();
    assert!(!rows.is_empty(), "付费采集应入库: {listed}");

    let before = invoke(host, "paid_gateway", "reserve", json!({})).await;
    assert_eq!(before.get("ok"), Some(&json!(true)));

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn push_listener_ingests_into_strata_read() {
    let _guard = BOOT_LOCK.lock().await;
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    for preset in builtin_presets() {
        write(config.path(), &format!("presets/{}", preset.file_name), preset.source);
    }
    write(config.path(), "system.yaml", base_system());
    let config_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config");
    write(
        config.path(),
        "accessors/push.yaml",
        &fs::read_to_string(config_root.join("accessors/push.yaml")).unwrap(),
    );
    write(
        config.path(),
        "sources/inbound-items.yaml",
        &fs::read_to_string(config_root.join("sources/inbound-items.yaml")).unwrap(),
    );
    write(
        config.path(),
        "adapters/json-list.yaml",
        &fs::read_to_string(config_root.join("adapters/json-list.yaml")).unwrap(),
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
    let push_addr = running.push();
    let body = r#"{"items":[{"url":"https://push.local/x","title":"Push One","publishedAt":"2024-01-01T00:00:00Z"}]}"#;
    let request = format!(
        "POST /items HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer local-inbound-token\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let mut stream = tokio::net::TcpStream::connect(push_addr).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let text = String::from_utf8_lossy(&response);
    assert!(text.contains("200"), "push 应接受: {text}");

    invoke(running.host(), "committer", "flush", json!({})).await;
    let listed = invoke(
        running.host(),
        "strata_read",
        "query",
        json!({
            "query": {
                "from": "resources",
                "limit": 20
            }
        }),
    )
    .await;
    let rows = listed.get("rows").and_then(|v| v.as_array()).unwrap();
    assert!(!rows.is_empty(), "push 入库后 strata_read 应能查到: {listed}");
    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn hotlist_apply_publishes_snapshot_rows() {
    let _guard = BOOT_LOCK.lock().await;
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

    let delta = invoke(host, "type_hotlist", "on_commit", json!({})).await;
    let applied = invoke(host, "committer", "hotlist_apply", json!({})).await;
    assert_eq!(
        applied.get("applied").and_then(|v| v.as_bool()),
        Some(!delta.is_null()),
        "hotlist_apply 应与 on_commit 差量一致: delta={delta} applied={applied}"
    );
    if delta.is_null() {
        running.shutdown().await.unwrap();
        return;
    }
    assert!(
        delta
            .pointer("/ops/0/variant")
            .and_then(|v| v.as_str())
            == Some("hotlist.delta"),
        "热榜差量应带 hotlist.delta: {delta}"
    );
    invoke(host, "committer", "flush", json!({})).await;
    let page = invoke(
        host,
        "strata_read",
        "query",
        json!({
            "query": {
                "from": "resources",
                "hasFacets": [
                    { "kind": "struct", "variant": "hotlist.delta" }
                ],
                "limit": 10
            }
        }),
    )
    .await;
    let rows = page.get("rows").and_then(|v| v.as_array()).unwrap();
    assert!(!rows.is_empty(), "热榜差量 facet 应可见: {page}");
    running.shutdown().await.unwrap();
}
