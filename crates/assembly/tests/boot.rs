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
