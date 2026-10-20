//! 组合根能拉起存储、配置代数，并只在运维端口放行只读调度面。

use std::fs;
use std::path::Path;
use std::time::Duration;

use ability_support::{InstanceId, MethodId};
use assembly::Binds;
use config::builtin_presets;
use frame_core::{Gateway, InvokeRequest};
use modalprism_collector::boot;
use serde_json::json;
use strata_model::{AccessTask, Locator, QueueName, SourceId, TaskId, TaskReason, Timestamp};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

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

#[tokio::test]
async fn one_click_boot_serves_generation_and_releases_leases() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let running = boot(
        data.path(),
        config.path(),
        Binds {
            http: "127.0.0.1:0".parse().unwrap(),
            proxy: "127.0.0.1:0".parse().unwrap(),
            push: "127.0.0.1:0".parse().unwrap(),
        },
    )
    .await
    .unwrap();
    assert!(data.path().join("control.db").is_file());
    assert!(data.path().join("lance").is_dir());
    assert!(data.path().join("staging").is_dir());

    let response = running
        .host()
        .invoke(InvokeRequest::new(
            InstanceId::new("config").unwrap(),
            MethodId::new("generation").unwrap(),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(response.data["generation"], 1);

    let mut stream = TcpStream::connect(running.http()).await.unwrap();
    stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut body = Vec::new();
    stream.read_to_end(&mut body).await.unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("ok"));

    let catalog = running.gateway().catalog();
    assert!(
        catalog
            .iter()
            .any(|entry| entry.instance.as_str() == "planner")
    );
    assert!(
        catalog
            .iter()
            .all(|entry| entry.instance.as_str() != "strata_write")
    );
    let denied = running
        .gateway()
        .invoke(InvokeRequest::new(
            InstanceId::new("strata_write").unwrap(),
            MethodId::new("apply").unwrap(),
            json!({}),
        ))
        .await;
    assert!(denied.is_err());

    let task_id = TaskId::new("task-boot").unwrap();
    let task = AccessTask {
        task_id: task_id.clone(),
        source_id: SourceId::new("src").unwrap(),
        accessor: "accessor/http".into(),
        locator: Locator::Url {
            url: "https://example.com/a".into(),
        },
        spec: json!({}),
        reason: TaskReason::Manual,
        priority: 1,
        queue: QueueName::Free,
        lease_until: Timestamp::now(),
        attempt: 0,
        batch_key: None,
        generation: 1,
    };
    running
        .services()
        .strata()
        .enqueue(std::slice::from_ref(&task))
        .await
        .unwrap();
    running
        .services()
        .strata()
        .lease_id(&task_id, Duration::from_secs(30), Timestamp::now())
        .await
        .unwrap();
    let root = data.path().to_path_buf();
    running.shutdown().await.unwrap();
    let again = strata_client::Strata::open(&root, strata_client::Options::default())
        .await
        .unwrap();
    assert_eq!(
        again
            .leased_count(QueueName::Free, Timestamp::now())
            .await
            .unwrap(),
        0
    );
}
