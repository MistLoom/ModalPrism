//! 组合根能拉起存储、配置代数，并只在运维端口放行只读调度面。

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use ability_support::{InstanceId, MethodId};
use collector_runtime::Binds;
use config::builtin_presets;
use frame_core::{Gateway, InvokeRequest};
use modalprism_collector::{apply_hotlist, boot};
use serde_json::json;
use strata_model::{
    AccessTask, ApplyOpts, Body, ChangeSet, FacetKind, FacetSelector, LineageInput, Locator, Op,
    Producer, Query, QueryFrom, QueueName, Rid, SourceId, Target, TaskId, TaskReason, Timestamp,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

static BOOT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
    let _guard = BOOT_LOCK.lock().await;
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
        modalprism_collector::BootOptions::default(),
    )
    .await
    .unwrap();
    assert!(data.path().join("control.db").is_file());
    assert!(data.path().join("lance").is_dir());
    assert!(data.path().join("staging").is_dir());

    let generation = running
        .host()
        .invoke(InvokeRequest::new(
            InstanceId::new("config").unwrap(),
            MethodId::new("generation").unwrap(),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(generation.data["generation"], 1);

    let mut stream = TcpStream::connect(running.http()).await.unwrap();
    stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut body = Vec::new();
    stream.read_to_end(&mut body).await.unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("ok"), "{text}");
    assert!(
        !text.to_ascii_lowercase().contains("access-control-"),
        "{text}"
    );

    let mut options = TcpStream::connect(running.http()).await.unwrap();
    options
        .write_all(
            b"OPTIONS /health HTTP/1.1\r\nHost: localhost\r\nOrigin: https://evil.example\r\nAccess-Control-Request-Method: GET\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();
    let mut option_body = Vec::new();
    options.read_to_end(&mut option_body).await.unwrap();
    let option_text = String::from_utf8_lossy(&option_body);
    assert!(
        !option_text.to_ascii_lowercase().contains("access-control-"),
        "{option_text}"
    );

    let catalog = running.gateway().catalog();
    assert!(
        catalog
            .iter()
            .any(|entry| entry.instance.as_str() == "planner")
    );
    let denied = running
        .http_ops_gateway()
        .invoke(InvokeRequest::new(
            InstanceId::new("strata_write").unwrap(),
            MethodId::new("apply").unwrap(),
            json!({}),
        ))
        .await;
    assert!(denied.is_err());
    for blocked in ["strata_queue", "committer", "runner", "egress"] {
        let denied = running
            .http_ops_gateway()
            .invoke(InvokeRequest::new(
                InstanceId::new(blocked).unwrap(),
                MethodId::new("flush").unwrap(),
                json!({}),
            ))
            .await;
        assert!(denied.is_err(), "{blocked} 应被 HTTP 表面拒绝");
    }

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
        .gateway()
        .invoke(InvokeRequest::new(
            InstanceId::new("strata_queue").unwrap(),
            MethodId::new("enqueue").unwrap(),
            json!({ "tasks": [task] }),
        ))
        .await
        .unwrap();
    running
        .gateway()
        .invoke(InvokeRequest::new(
            InstanceId::new("strata_queue").unwrap(),
            MethodId::new("lease_id").unwrap(),
            json!({
                "taskId": task_id.to_string(),
                "leaseMs": 30_000,
                "now": Timestamp::now().to_string(),
            }),
        ))
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

#[tokio::test]
async fn visible_flush_commits_the_hotlist_changeset() {
    let _guard = BOOT_LOCK.lock().await;
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    write(
        config.path(),
        "types/hotlist.yaml",
        "id: type/hotlist\nkind: type\nname: HotList\nspec:\n  keepSnapshots: 2\n",
    );
    let running = boot(
        data.path(),
        config.path(),
        binds_zero(),
        modalprism_collector::BootOptions::default(),
    )
        .await
        .unwrap();
    let strata = strata_client::Strata::open(data.path(), strata_client::Options::default())
        .await
        .unwrap();
    store(
        &strata,
        vec![ensure("ext:list:board", "list"), ensure("ext:item:a", "a")],
    )
    .await;
    strata.flush().await.unwrap();
    let list = strata.resolve("ext:list:board").await.unwrap().unwrap();
    let item = strata.resolve("ext:item:a").await.unwrap().unwrap();
    store(&strata, snapshot("ext:snap:1", "s1", &list, &item, 1)).await;
    strata.flush().await.unwrap();
    let first = strata
        .resolve("ext:snap:1")
        .await
        .unwrap()
        .unwrap()
        .to_string();
    let page = strata.query(hotlist_selector()).await.unwrap();
    let stamp = page.rows.iter().find(|row| row["rid"] == first).unwrap()["createdAt"]
        .as_str()
        .unwrap()
        .to_string();
    wait_past(&stamp).await;
    store(&strata, snapshot("ext:snap:2", "s2", &list, &item, 2)).await;
    strata.flush().await.unwrap();
    assert!(apply_hotlist(running.host()).await.unwrap());
    let listed = running
        .host()
        .invoke(InvokeRequest::new(
            InstanceId::new("strata_read").unwrap(),
            MethodId::new("query").unwrap(),
            json!({
                "query": {
                    "from": "resources",
                    "hasFacets": [{ "kind": "struct", "variant": "hotlist.delta" }]
                }
            }),
        ))
        .await
        .unwrap();
    assert!(!listed.data["rows"].as_array().unwrap().is_empty());
}

fn binds_zero() -> Binds {
    Binds {
        http: "127.0.0.1:0".parse().unwrap(),
        proxy: "127.0.0.1:0".parse().unwrap(),
        push: "127.0.0.1:0".parse().unwrap(),
    }
}

async fn wait_past(stamp: &str) {
    let parsed = Timestamp::parse(stamp).unwrap();
    for _ in 0..100 {
        if Timestamp::now() > parsed {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("时间没有前进");
}

async fn store(strata: &strata_client::Strata, ops: Vec<Op>) {
    strata
        .apply(
            ChangeSet {
                producer: Producer {
                    module: "test".into(),
                    version: "1".into(),
                    config_generation: 1,
                },
                idempotency_key: None,
                ack: None,
                ops,
            },
            ApplyOpts::default(),
        )
        .await
        .unwrap();
}

fn ensure(identity: &str, alias: &str) -> Op {
    Op::Ensure {
        identity: identity.into(),
        ns: None,
        labels: BTreeMap::new(),
        r#as: alias.into(),
    }
}

fn snapshot(identity: &str, alias: &str, list: &Rid, item: &Rid, rank: i64) -> Vec<Op> {
    vec![
        ensure(identity, alias),
        Op::PutFacet {
            target: Target::Alias(alias.into()),
            kind: FacetKind::Struct,
            variant: Some("hotlist.snapshot".into()),
            body: Body::Struct {
                value: json!({ "id": identity }),
            },
            media_type: Some("application/json".into()),
            lineage: LineageInput {
                inputs: Vec::new(),
                params_hash: "params".into(),
            },
        },
        Op::Link {
            src: Target::Alias(alias.into()),
            rel: "part_of".into(),
            dst: Target::Rid(list.clone()),
            attrs: None,
        },
        Op::Link {
            src: Target::Alias(alias.into()),
            rel: "entry".into(),
            dst: Target::Rid(item.clone()),
            attrs: Some(json!({ "rank": rank })),
        },
    ]
}

fn hotlist_selector() -> Query {
    Query {
        from: QueryFrom::Resources,
        r#where: None,
        has_facets: vec![FacetSelector {
            kind: FacetKind::Struct,
            variant: Some("hotlist.snapshot".into()),
            fields: Vec::new(),
        }],
        select: Vec::new(),
        expand: Vec::new(),
        search: None,
        as_of: None,
        order_by: Vec::new(),
        limit: None,
        cursor: None,
    }
}
