//! 采集链路：本地 HTTP RSS 夹具 → planner → runner → strata_write → flush → strata_read。

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

const RSS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0"><channel>
<title>Fixture</title>
<link>http://fixture.local/</link>
<item><title>One</title><link>http://fixture.local/a</link><pubDate>Mon, 01 Jan 2024 00:00:00 GMT</pubDate></item>
<item><title>Two</title><link>http://fixture.local/b</link><pubDate>Tue, 02 Jan 2024 00:00:00 GMT</pubDate></item>
</channel></rss>"#;

fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn scaffold(config: &Path, feed_url: &str) {
    for preset in builtin_presets() {
        write(config, &format!("presets/{}", preset.file_name), preset.source);
    }
    write(
        config,
        "system.yaml",
        "id: system\nkind: system\nname: ModalPrism\npolicy:\n  preset: standard\n  egress:\n    private_networks: allow\n",
    );
    let config_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config");
    write(
        config,
        "accessors/rss.yaml",
        &fs::read_to_string(config_root.join("accessors/rss.yaml")).unwrap(),
    );
    write(
        config,
        "sources/fixture-rss.yaml",
        &format!(
            r#"id: source/fixture-rss
kind: source
name: Fixture RSS
policy:
  robots:
    mode: ignore
    robots_reason: integration-test
access:
  accessor: accessor/rss
  locator:
    url: {feed_url}
pipeline: []
schedule:
  interval: 1h
admission:
  initialBackfill:
    limit: 10
    maxAge: 365d
"#
        ),
    );
}

async fn spawn_rss_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let body = RSS.to_string();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/rss+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf).await;
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    port
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

#[tokio::test]
async fn one_round_ingests_fixture_rss_into_strata_read() {
    let _guard = BOOT_LOCK.lock().await;
    let port = spawn_rss_server().await;
    let feed_url = format!("http://127.0.0.1:{port}/feed.xml");
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    scaffold(config.path(), &feed_url);

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

    let tasks = invoke(
        host,
        "planner",
        "trigger",
        json!({ "source": "fixture-rss" }),
    )
    .await;
    let tasks = if tasks.as_array().is_some_and(|a| !a.is_empty()) {
        tasks
    } else {
        invoke(host, "planner", "tick", json!({})).await
    };
    let tasks = tasks.as_array().expect("任务列表");
    assert!(!tasks.is_empty(), "planner 应派出任务，tick/trigger 返回: {tasks:?}");
    for task in tasks {
        let outcome = invoke(host, "runner", "run_task", json!({ "task": task.clone() })).await;
        if outcome.get("nacked").is_some() {
            panic!("run_task 被拒绝: {outcome:?}");
        }
        assert!(
            outcome.get("ok").and_then(|v| v.as_bool()) == Some(true),
            "run_task 应成功: {outcome:?}"
        );
    }
    let flushed = invoke(host, "committer", "flush", json!({})).await;
    assert!(
        flushed.get("snapshot").is_some_and(|s| !s.is_null()),
        "flush 应发布快照: {flushed:?}"
    );
    let listed = invoke(
        host,
        "strata_read",
        "query",
        json!({
            "query": {
                "from": "resources",
                "limit": 50
            }
        }),
    )
    .await;
    let rows = listed.get("rows").and_then(|v| v.as_array()).unwrap();
    assert!(
        !rows.is_empty(),
        "一轮采集后 strata_read 应能查到资源，实际: {listed}"
    );
    running.shutdown().await.unwrap();
}
