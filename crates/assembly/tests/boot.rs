//! 空目录启动、快照保留、配置失败和租约释放。

use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use assembly::{Binds, Services};
use config::builtin_presets;
use serde_json::json;
use strata_model::{
    AccessTask, ApplyOpts, Body, ChangeSet, FacetKind, LineageInput, Locator, Op, Producer,
    QueueName, SourceId, Target, TaskId, TaskReason, Timestamp,
};

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
        "id: system\nkind: system\nname: ModalPrism\npolicy:\n  preset: standard\n",
    );
}

fn binds() -> Binds {
    Binds {
        http: "127.0.0.1:0".parse().unwrap(),
        proxy: "127.0.0.1:0".parse().unwrap(),
        push: "127.0.0.1:0".parse().unwrap(),
    }
}

fn producer() -> Producer {
    Producer {
        module: "assembly-test".into(),
        version: "1".into(),
        config_generation: 1,
    }
}

#[tokio::test]
async fn empty_dir_creates_storage_and_keeps_snapshot() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let services = Services::open(data.path(), config.path(), binds())
        .await
        .unwrap();
    assert!(data.path().join("control.db").is_file());
    assert!(data.path().join("lance").is_dir());
    assert!(data.path().join("staging").is_dir());
    let applied = services
        .strata()
        .apply(
            ChangeSet {
                producer: producer(),
                idempotency_key: None,
                ack: None,
                ops: vec![
                    Op::Ensure {
                        identity: "ext:doi:10.1000/a".into(),
                        ns: None,
                        labels: Default::default(),
                        r#as: "a".into(),
                    },
                    Op::PutFacet {
                        target: Target::Alias("a".into()),
                        kind: FacetKind::Text,
                        variant: None,
                        body: Body::Text { text: "one".into() },
                        media_type: Some("text/plain".into()),
                        lineage: LineageInput {
                            inputs: Vec::new(),
                            params_hash: "raw".into(),
                        },
                    },
                ],
            },
            ApplyOpts::default(),
        )
        .await
        .unwrap();
    assert!(applied.snapshot.is_none());
    services.call("committer.flush", json!({})).await.unwrap();
    let rid = services
        .strata()
        .resolve("ext:doi:10.1000/a")
        .await
        .unwrap()
        .unwrap();
    drop(services);
    let again = Services::open(data.path(), config.path(), binds())
        .await
        .unwrap();
    assert_eq!(
        again
            .strata()
            .resolve("ext:doi:10.1000/a")
            .await
            .unwrap()
            .unwrap(),
        rid
    );
}

#[tokio::test]
async fn failed_reload_keeps_generation() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let services = Services::open(data.path(), config.path(), binds())
        .await
        .unwrap();
    let before = services.call("config.generation", json!({})).await.unwrap();
    fs::write(config.path().join("system.yaml"), "kind: [\n").unwrap();
    assert!(services.call("config.load", json!({})).await.is_err());
    let after = services.call("config.generation", json!({})).await.unwrap();
    assert_eq!(before, after);
    assert!(
        services
            .call("config_inspect.validate", json!({}))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn remote_store_and_foreign_address_do_not_create_data() {
    let data = tempfile::tempdir().unwrap();
    let nested = data.path().join("empty");
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    write(
        config.path(),
        "system.yaml",
        "id: system\nkind: system\nname: ModalPrism\nstrata:\n  control_store:\n    backend: postgres\npolicy:\n  preset: standard\n",
    );
    let Err(error) = Services::open(&nested, config.path(), binds()).await else {
        panic!("远程存储应当拒绝启动");
    };
    assert_eq!(error.to_string(), "存储位置未规定");
    assert!(!nested.join("control.db").exists());

    config_dir(config.path());
    let bad = Binds {
        http: "0.0.0.0:9".parse::<SocketAddr>().unwrap(),
        proxy: "127.0.0.1:0".parse().unwrap(),
        push: "127.0.0.1:0".parse().unwrap(),
    };
    let Err(error) = Services::open(&nested, config.path(), bad).await else {
        panic!("非环回地址应当拒绝启动");
    };
    assert_eq!(error.to_string(), "监听地址不是环回地址");
    assert!(!nested.exists());
}

#[tokio::test]
async fn shutdown_releases_leases() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let services = Services::open(data.path(), config.path(), binds())
        .await
        .unwrap();
    let task_id = TaskId::new("task-1").unwrap();
    let task = AccessTask {
        task_id: task_id.clone(),
        source_id: SourceId::new("src").unwrap(),
        accessor: "accessor/http".into(),
        locator: Locator::Url {
            url: "https://example.com/a".into(),
        },
        spec: json!({}),
        reason: TaskReason::Manual,
        priority: 2,
        queue: QueueName::Free,
        lease_until: Timestamp::now(),
        attempt: 0,
        batch_key: None,
        generation: 1,
    };
    services
        .strata()
        .enqueue(std::slice::from_ref(&task))
        .await
        .unwrap();
    services
        .strata()
        .lease_id(&task_id, Duration::from_secs(60), Timestamp::now())
        .await
        .unwrap();
    assert_eq!(
        services
            .strata()
            .leased_count(QueueName::Free, Timestamp::now())
            .await
            .unwrap(),
        1
    );
    services.shutdown().await.unwrap();
    assert_eq!(
        services
            .strata()
            .leased_count(QueueName::Free, Timestamp::now())
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn standard_preset_redacts_headers_before_they_reach_staging() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    write(
        config.path(),
        "system.yaml",
        "id: system\nkind: system\nname: ModalPrism\npolicy:\n  preset: standard\n  egress:\n    private_networks: allow\nstrata:\n  blob:\n    inline_size_threshold: 32KiB\n    dedicated_size_threshold: 8MiB\n  staging:\n    ttl: 12h\n",
    );
    write(
        config.path(),
        "accessors/browser.yaml",
        "id: accessor/browser\nkind: accessor\nname: Browser\nimplementation: builtin:browser\nlimits:\n  timeout: 30s\n  concurrency: 2\n  retry:\n    attempts: 1\n    backoff: exponential\n    base: 1s\n    max: 30s\negress:\n  profile: direct\n",
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buf).await;
                let text = String::from_utf8_lossy(&buf);
                let body = if text.contains("robots.txt") {
                    "User-agent: *\nAllow: /\n"
                } else {
                    "ok"
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = tokio::io::AsyncWriteExt::write_all(&mut socket, response.as_bytes()).await;
            });
        }
    });
    write(
        config.path(),
        "sources/demo.yaml",
        &format!(
            "id: source/demo\nkind: source\nname: Demo\naccess:\n  accessor: accessor/browser\n  locator:\n    url: http://{addr}/page\npipeline: []\nschedule:\n  interval: 15m\nadmission: {{}}\n"
        ),
    );
    let script = data.path().join("moli.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\ncat <<'EOF'\n{\"final_url\":\"http://127.0.0.1/page\",\"status\":200,\"headers\":[[\"Content-Type\",\"text/html\"],[\"Set-Cookie\",\"secret\"]],\"html\":\"<p>ok</p>\",\"version\":\"fixture\"}\nEOF\n",
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&script).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
    std::fs::set_permissions(&script, permissions).unwrap();
    let services = Services::open(data.path(), config.path(), binds())
        .await
        .unwrap();
    services.set_browser_program(&script);
    assert_eq!(
        assembly::snapshot_interval(&services.config().applied()),
        Duration::from_secs(15 * 60)
    );
    let task = AccessTask {
        task_id: TaskId::new("task-redact").unwrap(),
        source_id: SourceId::new("source/demo").unwrap(),
        accessor: "accessor/browser".into(),
        locator: Locator::Url {
            url: format!("http://{addr}/page"),
        },
        spec: json!({}),
        reason: TaskReason::Manual,
        priority: 10,
        queue: QueueName::Render,
        lease_until: Timestamp::now(),
        attempt: 0,
        batch_key: None,
        generation: 4,
    };
    services
        .strata()
        .enqueue(std::slice::from_ref(&task))
        .await
        .unwrap();
    services
        .strata()
        .lease_id(&task.task_id, Duration::from_secs(60), Timestamp::now())
        .await
        .unwrap();
    services
        .call(
            "runner.run_task",
            json!({ "task": serde_json::to_value(&task).unwrap() }),
        )
        .await
        .unwrap();
    let mut saw_redacted = false;
    for entry in std::fs::read_dir(data.path().join("staging")).unwrap() {
        let path = entry.unwrap().path();
        if !path.is_file() {
            continue;
        }
        let text = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
        assert!(!text.contains("secret"), "{text}");
        if text.contains("***") {
            saw_redacted = true;
        }
    }
    assert!(saw_redacted);
}
