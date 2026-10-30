//! 空目录启动、快照保留、配置失败和租约释放。

use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

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
    assert!(data.path().join("gateway.db").is_file());
    // 测试固定用本地目录后端，不下载 RustFS 发行包。
    assert!(data.path().join("blobs").is_dir());
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
    services.committer_flush().await.unwrap();
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
    let before = services.config_generation();
    fs::write(config.path().join("system.yaml"), "kind: [\n").unwrap();
    assert!(services.config_reload().is_err());
    let after = services.config_generation();
    assert_eq!(before, after);
    assert!(
        types::read::call(services.as_ref(), "config_inspect.validate", json!({}))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn postgres_without_url_and_foreign_address_do_not_create_data() {
    let data = tempfile::tempdir().unwrap();
    let nested = data.path().join("empty");
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    write(
        config.path(),
        "system.yaml",
        "id: system\nkind: system\nname: ModalPrism\nstrata:\n  gateway:\n    backend: postgres\npolicy:\n  preset: standard\n",
    );
    let Err(error) = Services::open(&nested, config.path(), binds()).await else {
        panic!("缺少连接串的 PostgreSQL 应当拒绝启动");
    };
    assert_eq!(error.to_string(), "存储位置未规定");
    assert!(!nested.join("gateway.db").exists());

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
        plan_id: None,
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
        "id: system\nkind: system\nname: ModalPrism\npolicy:\n  preset: standard\n  egress:\n    private_networks: allow\nstrata:\n  gateway:\n    blob_backend: dir\n  blob:\n    inline_size_threshold: 32KiB\n    dedicated_size_threshold: 8MiB\n  staging:\n    ttl: 12h\n",
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
    // 批处理里 < > 是重定向符，JSON 中的 HTML 片段需要用 ^ 转义。
    #[cfg(windows)]
    let script = {
        let script = data.path().join("moli.cmd");
        std::fs::write(
            &script,
            "@echo off\necho {\"final_url\":\"http://127.0.0.1/page\",\"status\":200,\"headers\":[[\"Content-Type\",\"text/html\"],[\"Set-Cookie\",\"secret\"]],\"html\":\"^<p^>ok^</p^>\",\"version\":\"fixture\"}\n",
        )
        .unwrap();
        script
    };
    #[cfg(unix)]
    let script = data.path().join("moli.sh");
    #[cfg(unix)]
    std::fs::write(
        &script,
        "#!/bin/sh\ncat <<'EOF'\n{\"final_url\":\"http://127.0.0.1/page\",\"status\":200,\"headers\":[[\"Content-Type\",\"text/html\"],[\"Set-Cookie\",\"secret\"]],\"html\":\"<p>ok</p>\",\"version\":\"fixture\"}\nEOF\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script, permissions).unwrap();
    }
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
        plan_id: None,
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
    services.runner_run(task).await.unwrap();
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

fn http_limit(dir: &Path, timeout: &str, addr: SocketAddr) {
    write(
        dir,
        "system.yaml",
        "id: system\nkind: system\nname: ModalPrism\npolicy:\n  preset: standard\n  egress:\n    private_networks: allow\nstrata:\n  gateway:\n    blob_backend: dir\n",
    );
    write(
        dir,
        "accessors/http.yaml",
        &format!(
            "id: accessor/http\nkind: accessor\nname: HTTP\nimplementation: builtin:http\nlimits:\n  timeout: {timeout}\n  concurrency: 4\n  retry:\n    attempts: 1\n    backoff: exponential\n    base: 1s\n    max: 30s\negress:\n  profile: direct\n"
        ),
    );
    write(
        dir,
        "sources/slow.yaml",
        &format!(
            "id: source/slow\nkind: source\nname: Slow\npolicy:\n  robots:\n    mode: ignore\n    robots_reason: local-slow-server\naccess:\n  accessor: accessor/http\n  locator:\n    url: http://{addr}/slow\npipeline: []\nschedule:\n  interval: 15m\nadmission: {{}}\n"
        ),
    );
}

async fn slow_page(delay: Duration) -> (SocketAddr, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&hits);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let count = Arc::clone(&count);
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buf).await;
                let text = String::from_utf8_lossy(&buf);
                let body = if text.contains("robots.txt") {
                    "User-agent: *\nAllow: /\n".to_string()
                } else {
                    count.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(delay).await;
                    "slow-ok".to_string()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = tokio::io::AsyncWriteExt::write_all(&mut socket, response.as_bytes()).await;
            });
        }
    });
    (addr, hits)
}

fn assert_lease(tasks: &serde_json::Value, before: Timestamp, after: Timestamp, timeout: Duration) {
    let span = timeout.saturating_mul(3) / 2;
    let lease = Timestamp::parse(tasks[0]["leaseUntil"].as_str().unwrap()).unwrap();
    let low = before.checked_add(span).unwrap();
    let high = after.checked_add(span).unwrap();
    assert!(
        lease >= low && lease <= high,
        "租约 {lease} 应落在 {low} 与 {high} 之间"
    );
}

async fn run_slow(
    timeout_text: &str,
    timeout: Duration,
    delay: Duration,
) -> (Duration, bool, usize) {
    let (addr, hits) = slow_page(delay).await;
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    http_limit(config.path(), timeout_text, addr);
    let services = Services::open(data.path(), config.path(), binds())
        .await
        .unwrap();
    let before = Timestamp::now();
    let tasks = serde_json::to_value(services.planner_tick().await.unwrap()).unwrap();
    let after = Timestamp::now();
    assert_eq!(tasks.as_array().map(Vec::len), Some(1));
    assert_lease(&tasks, before, after, timeout);
    let started = std::time::Instant::now();
    services
        .runner_run(serde_json::from_value(tasks[0].clone()).unwrap())
        .await
        .unwrap();
    let elapsed = started.elapsed();
    services.committer_flush().await.unwrap();
    let stored = services
        .strata()
        .resolve(&format!("url:http://{addr}/slow"))
        .await
        .unwrap();
    (elapsed, stored.is_some(), hits.load(Ordering::SeqCst))
}

#[tokio::test]
async fn push_stores_items_that_can_be_read_back() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    write(
        config.path(),
        "accessors/push.yaml",
        "id: accessor/push\nkind: accessor\nname: Push\nimplementation: builtin:push\nlimits:\n  timeout: 30s\n  concurrency: 1\n  retry:\n    attempts: 0\n    backoff: fixed\n    base: 1s\n    max: 1s\negress:\n  profile: direct\n",
    );
    write(
        config.path(),
        "adapters/json-list.yaml",
        "id: adapter/json-list\nkind: adapter\nname: JSON\nimplementation: builtin:json-list\nrole: search\nconsumes:\n  - {kind: raw, media_type: application/json}\nproduces: [discovery]\nlimits:\n  timeout: 30s\n  concurrency: 1\n  retry:\n    attempts: 0\n    backoff: fixed\n    base: 1s\n    max: 1s\n",
    );
    write(
        config.path(),
        "accessors/http.yaml",
        "id: accessor/http\nkind: accessor\nname: HTTP\nimplementation: builtin:http\nlimits:\n  timeout: 30s\n  concurrency: 1\n  retry:\n    attempts: 0\n    backoff: fixed\n    base: 1s\n    max: 1s\negress:\n  profile: direct\n",
    );
    write(
        config.path(),
        "sources/ahead.yaml",
        "id: source/ahead\nkind: source\nname: Ahead\naccess:\n  accessor: accessor/http\n  locator:\n    url: https://example.com/list\npipeline: []\nschedule:\n  interval: 1h\nadmission: {}\n",
    );
    write(
        config.path(),
        "sources/inbound.yaml",
        "id: source/inbound-items\nkind: source\nname: Inbound\naccess:\n  accessor: accessor/push\n  locator:\n    channel: items\n  spec:\n    token: local-inbound-token\npipeline:\n  - run: [adapter/json-list]\n    spec:\n      adapter/json-list:\n        items: /items\n        url: url\n        title: title\nschedule:\n  interval: 1h\nadmission: {}\n",
    );
    let services = Services::open(data.path(), config.path(), binds())
        .await
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let background = Arc::clone(&services);
    tokio::spawn(async move {
        assembly::serve_push(background, listener).await;
    });
    let body = r#"{"items":[{"title":"推送标题","url":"https://example.com/pushed"}]}"#;
    let request = format!(
        "POST /items HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer local-inbound-token\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let text = String::from_utf8_lossy(&response);
    assert!(text.starts_with("HTTP/1.1 200"), "{text}");
    let resolved = services
        .strata()
        .resolve("url:https://example.com/pushed")
        .await
        .unwrap();
    assert!(resolved.is_some(), "{text}");
}

#[tokio::test]
async fn detail_follow_stores_the_missing_date_and_skips_the_filled_item() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let detail_b = Arc::new(AtomicUsize::new(0));
    let detail_bg = Arc::clone(&detail_b);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let detail_bg = Arc::clone(&detail_bg);
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let text = String::from_utf8_lossy(&buf[..n]);
                let path = text.split_whitespace().nth(1).unwrap_or("/");
                let (ctype, body) = if path.contains("robots.txt") {
                    ("text/plain", "User-agent: *\nAllow: /\n".to_string())
                } else if path.starts_with("/list") {
                    (
                        "application/json",
                        format!(
                            r#"{{"items":[{{"url":"http://{addr}/a","title":"甲"}},{{"url":"http://{addr}/b","title":"乙","publishedAt":"2026-10-01T00:00:00Z"}}]}}"#
                        ),
                    )
                } else if path.starts_with("/a") {
                    (
                        "text/html",
                        r#"<time class="hero-date" datetime="2026-09-27">September 27, 2026</time><h1 class="hero-title">文章</h1>"#.to_string(),
                    )
                } else if path.starts_with("/b") {
                    detail_bg.fetch_add(1, Ordering::SeqCst);
                    ("text/html", "filled".to_string())
                } else {
                    ("text/plain", "missing".to_string())
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    write(
        config.path(),
        "system.yaml",
        "id: system\nkind: system\nname: ModalPrism\npolicy:\n  preset: standard\n  egress:\n    private_networks: allow\nstrata:\n  gateway:\n    blob_backend: dir\n",
    );
    write(
        config.path(),
        "accessors/http.yaml",
        "id: accessor/http\nkind: accessor\nname: HTTP\nimplementation: builtin:http\nlimits:\n  timeout: 30s\n  concurrency: 4\n  retry:\n    attempts: 1\n    backoff: exponential\n    base: 1s\n    max: 30s\negress:\n  profile: direct\n",
    );
    write(
        config.path(),
        "adapters/json-list.yaml",
        "id: adapter/json-list\nkind: adapter\nname: JSON\nimplementation: builtin:json-list\nrole: search\nconsumes:\n  - {kind: raw, media_type: application/json}\nproduces: [discovery]\nlimits:\n  timeout: 30s\n  concurrency: 1\n  retry:\n    attempts: 0\n    backoff: fixed\n    base: 1s\n    max: 1s\n",
    );
    write(
        config.path(),
        "adapters/page-fields.yaml",
        "id: adapter/page-fields\nkind: adapter\nname: 页面字段\nimplementation: builtin:page-fields\nrole: content\nconsumes:\n  - {kind: raw, part_role: document, media_type: text/html}\nproduces:\n  - {kind: struct, variant: page}\nlimits:\n  timeout: 30s\n  concurrency: 1\n  retry:\n    attempts: 0\n    backoff: fixed\n    base: 1s\n    max: 1s\n",
    );
    write(
        config.path(),
        "sources/detail.yaml",
        &format!(
            "id: source/detail\nkind: source\nname: Detail\npolicy:\n  robots:\n    mode: ignore\n    robots_reason: local-detail-server\naccess:\n  accessor: accessor/http\n  locator:\n    url: http://{addr}/list\npipeline:\n  - run: [adapter/json-list]\n    spec:\n      adapter/json-list:\n        items: /items\n        url: url\n        title: title\n        publishedAt: publishedAt\n        detail:\n          maxFetches: 1\n          fields: [publishedAt]\n  - for: follow\n    run: [adapter/page-fields]\n    spec:\n      adapter/page-fields:\n        publishedAt: time.hero-date\n        publishedAtAttr: datetime\n        title: h1.hero-title\nschedule:\n  interval: 1h\nadmission:\n  allowUrlPrefixes: [\"http://{addr}/\"]\n  initialBackfill:\n    limit: 10\n    maxAge: 365d\n"
        ),
    );
    let services = Services::open(data.path(), config.path(), binds())
        .await
        .unwrap();
    let tasks = serde_json::to_value(services.planner_tick().await.unwrap()).unwrap();
    assert_eq!(tasks.as_array().map(Vec::len), Some(1));
    let ran = services
        .runner_run(serde_json::from_value(tasks[0].clone()).unwrap())
        .await
        .unwrap();
    let follows = serde_json::to_value(&ran.follows).unwrap();
    assert_eq!(follows.as_array().map(Vec::len), Some(1));
    let locator = follows[0]["locator"]["url"].as_str().unwrap();
    assert!(locator.ends_with("/a"), "{locator}");
    services.committer_flush().await.unwrap();
    services
        .runner_run(serde_json::from_value(follows[0].clone()).unwrap())
        .await
        .unwrap();
    services.committer_flush().await.unwrap();
    assert_eq!(detail_b.load(Ordering::SeqCst), 0);
    let rid = services
        .strata()
        .resolve(&format!("url:http://{addr}/a"))
        .await
        .unwrap()
        .unwrap();
    let page = types::read::call(
        services.as_ref(),
        "strata_read.get",
        json!({ "facet": format!("{rid}@struct:page#latest") }),
    )
    .await
    .unwrap();
    assert_eq!(
        page.pointer("/struct/publishedAt")
            .and_then(|item| item.as_str()),
        Some("2026-09-27T00:00:00Z")
    );
    assert_eq!(
        page.pointer("/struct/title").and_then(|item| item.as_str()),
        Some("文章")
    );
}

#[tokio::test]
async fn short_limit_interrupts_before_thirty_seconds() {
    let (elapsed, stored, hits) =
        run_slow("2s", Duration::from_secs(2), Duration::from_secs(15)).await;
    assert!(hits >= 1, "慢服务器应接到请求");
    assert!(!stored);
    assert!(
        elapsed < Duration::from_secs(10),
        "2 秒时限应在 30 秒前中断，实际 {elapsed:?}"
    );
}

#[tokio::test]
async fn long_limit_keeps_the_request_past_thirty_seconds() {
    let (elapsed, stored, hits) =
        run_slow("45s", Duration::from_secs(45), Duration::from_secs(32)).await;
    assert!(hits >= 1, "慢服务器应接到请求");
    assert!(stored);
    assert!(
        elapsed > Duration::from_secs(30),
        "45 秒时限应留下超过 30 秒的响应，实际 {elapsed:?}"
    );
}
