//! 墓碑可见性：默认过滤、显式包含，以及全文检索与基础行两条路径的一致性。
//!
//! 别名只在同一个 changeset 内有效，因此写墓碑或恢复时一并重放 `Ensure`。

use std::collections::BTreeMap;

use strata_gateway::{BlobBackend, Gateway, Options, Request};
use strata_model::{
    ApplyOpts, Body, ChangeSet, FacetKind, LineageInput, Op, Producer, QueryFrom, Search, Target,
    WaitFor,
};

fn changeset(ops: Vec<Op>) -> ChangeSet {
    ChangeSet {
        producer: Producer {
            module: "query-it".into(),
            version: "1".into(),
            config_generation: 1,
        },
        idempotency_key: None,
        ack: None,
        ops,
    }
}

fn ensure(alias: &str) -> Op {
    Op::Ensure {
        identity: format!("ext:it:{alias}"),
        ns: None,
        labels: BTreeMap::new(),
        r#as: alias.into(),
    }
}

fn facet(alias: &str) -> Op {
    Op::PutFacet {
        target: Target::Alias(alias.into()),
        kind: FacetKind::Text,
        variant: None,
        body: Body::Text {
            text: "needle body".into(),
        },
        media_type: Some("text/plain".into()),
        lineage: LineageInput {
            inputs: Vec::new(),
            params_hash: "params".into(),
        },
    }
}

fn tombstone(alias: &str) -> Vec<Op> {
    vec![
        ensure(alias),
        Op::Tombstone {
            target: Target::Alias(alias.into()),
            reason: "测试删除".into(),
        },
    ]
}

fn restore(alias: &str) -> Vec<Op> {
    vec![
        ensure(alias),
        Op::Restore {
            target: Target::Alias(alias.into()),
            reason: "测试恢复".into(),
        },
    ]
}

async fn open() -> (tempfile::TempDir, Gateway) {
    let dir = tempfile::tempdir().unwrap();
    let gateway = Gateway::open(
        dir.path(),
        Options {
            blobs: BlobBackend::Memory,
            ..Options::default()
        },
    )
    .await
    .unwrap();
    // 两条资源各带一段可检索正文。
    flush(
        &gateway,
        vec![ensure("a"), facet("a"), ensure("b"), facet("b")],
    )
    .await;
    (dir, gateway)
}

async fn flush(gateway: &Gateway, ops: Vec<Op>) {
    let accepted = gateway
        .apply(changeset(ops), ApplyOpts::default())
        .await
        .unwrap();
    assert_eq!(accepted.status, WaitFor::Accepted);
    gateway.flush().await.unwrap();
}

async fn list(gateway: &Gateway, include_tombstoned: bool, search: Option<Search>) -> usize {
    let mut request = Request::new(QueryFrom::Resources);
    request.include_tombstoned = include_tombstoned;
    request.search = search;
    gateway.query(request).await.unwrap().rows.len()
}

fn needle() -> Search {
    Search {
        text: Some("needle".into()),
        vector: None,
        variant: None,
        k: 10,
    }
}

#[tokio::test]
async fn tombstoned_rows_are_hidden_by_default() {
    let (_dir, gateway) = open().await;
    assert_eq!(list(&gateway, false, None).await, 2);

    flush(&gateway, tombstone("a")).await;

    assert_eq!(list(&gateway, false, None).await, 1, "默认不应列出已删除");
    assert_eq!(list(&gateway, true, None).await, 2, "显式包含才返回墓碑");
}

#[tokio::test]
async fn search_path_hides_tombstones_like_the_row_path() {
    let (_dir, gateway) = open().await;
    assert_eq!(list(&gateway, false, Some(needle())).await, 2);

    flush(&gateway, tombstone("b")).await;

    // 全文检索直连索引，也必须和基础行路径给出同样的可见范围。
    assert_eq!(list(&gateway, false, Some(needle())).await, 1);
    assert_eq!(list(&gateway, true, Some(needle())).await, 2);
}

#[tokio::test]
async fn restore_brings_the_row_back() {
    let (_dir, gateway) = open().await;
    flush(&gateway, tombstone("a")).await;
    assert_eq!(list(&gateway, false, None).await, 1);

    flush(&gateway, restore("a")).await;

    assert_eq!(list(&gateway, false, None).await, 2);
    assert_eq!(list(&gateway, false, Some(needle())).await, 2);
}
