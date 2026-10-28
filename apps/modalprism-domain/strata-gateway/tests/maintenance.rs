//! 保留清理：版本下界、facet 保留版数、无引用 blob，以及 `dry_run` 不写。

use std::collections::BTreeMap;

use serde_json::json;
use strata_gateway::maintenance::PrunePlan;
use strata_gateway::{BlobBackend, Dialect, Gateway, Options, Sql};
use strata_model::{
    ApplyOpts, Body, ChangeSet, FacetKind, LineageInput, Op, Producer, Target, Timestamp, WaitFor,
};

fn changeset(ops: Vec<Op>) -> ChangeSet {
    ChangeSet {
        producer: Producer {
            module: "maintenance-it".into(),
            version: "1".into(),
            config_generation: 1,
        },
        idempotency_key: None,
        ack: None,
        ops,
    }
}

fn ensure() -> Op {
    Op::Ensure {
        identity: "ext:it:keep".into(),
        ns: None,
        labels: BTreeMap::new(),
        r#as: "keep".into(),
    }
}

fn facet(text: &str) -> Op {
    Op::PutFacet {
        target: Target::Alias("keep".into()),
        kind: FacetKind::Text,
        variant: None,
        body: Body::Text { text: text.into() },
        media_type: Some("text/plain".into()),
        lineage: LineageInput {
            inputs: Vec::new(),
            params_hash: "params".into(),
        },
    }
}

async fn flush(gateway: &Gateway, ops: Vec<Op>) {
    let accepted = gateway
        .apply(changeset(ops), ApplyOpts::default())
        .await
        .unwrap();
    assert_eq!(accepted.status, WaitFor::Accepted);
    gateway.flush().await.unwrap();
}

/// 直接写一行没有被任何 facet 引用的 blob，用来验证孤立存储行会被清掉。
async fn orphan_blob(gateway: &Gateway) {
    let mut sql = Sql::new(Dialect::Sqlite);
    sql.push(
        "INSERT INTO g_blobs(sha256, size, media_type, tier, inline, created_at, seq, v) VALUES (",
    );
    sql.text("orphan-orphan-orphan".to_string());
    sql.push(", 6, ");
    sql.text("text/plain".to_string());
    sql.push(", ");
    sql.text("inline".to_string());
    sql.push(", NULL, ");
    sql.text("2000-01-01T00:00:00Z".to_string());
    sql.push(", 0, 99)");
    gateway.db().exec(&sql).await.unwrap();
}

#[tokio::test]
async fn purge_content_clears_rows_objects_and_snapshots() {
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
    let part = gateway.stage(&vec![7u8; 70_000]).await.unwrap();
    flush(
        &gateway,
        vec![
            ensure(),
            facet("可清除正文"),
            Op::PutFacet {
                target: Target::Alias("keep".into()),
                kind: FacetKind::Raw,
                variant: Some("binary".into()),
                body: Body::Staged { part },
                media_type: Some("application/octet-stream".into()),
                lineage: LineageInput {
                    inputs: Vec::new(),
                    params_hash: "purge".into(),
                },
            },
        ],
    )
    .await;
    let before = gateway.store_counts().await.unwrap();
    assert_eq!(before.resources, 1);
    assert_eq!(before.facets, 2);
    assert_eq!(before.blobs, 1);
    assert_eq!(before.snapshots, 1);

    let report = gateway.purge_content().await.unwrap();
    assert_eq!(report.objects, 1);
    assert_eq!(report.tables.get("g_resources"), Some(&1));
    assert_eq!(report.tables.get("g_facets"), Some(&2));
    assert_eq!(report.tables.get("g_snapshots"), Some(&1));
    let after = gateway.store_counts().await.unwrap();
    assert_eq!(after.resources, 0);
    assert_eq!(after.facets, 0);
    assert_eq!(after.blobs, 0);
    assert_eq!(after.snapshots, 0);
    assert_eq!(gateway.view_latest().await.unwrap().resources.len(), 0);

    // 第二次清理幂等，不再有对象。
    let again = gateway.purge_content().await.unwrap();
    assert_eq!(again.objects, 0);
    assert_eq!(again.orphaned_objects, 0);
}

#[tokio::test]
async fn purge_derived_keeps_structured_content() {
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
    flush(
        &gateway,
        vec![
            ensure(),
            Op::PutFacet {
                target: Target::Alias("keep".into()),
                kind: FacetKind::Struct,
                variant: Some("article".into()),
                body: Body::Struct {
                    value: json!({"title": "保留"}),
                },
                media_type: Some("application/json".into()),
                lineage: LineageInput {
                    inputs: Vec::new(),
                    params_hash: "derive".into(),
                },
            },
            facet("派生正文"),
        ],
    )
    .await;
    assert_eq!(gateway.store_counts().await.unwrap().facets, 2);
    let report = gateway.purge_derived().await.unwrap();
    assert_eq!(report.tables.get("g_facets"), Some(&1));
    let counts = gateway.store_counts().await.unwrap();
    assert_eq!(counts.resources, 1);
    assert_eq!(counts.facets, 1, "结构化 facet 必须保留");
    assert_eq!(gateway.view_latest().await.unwrap().facets.len(), 1);
}

#[tokio::test]
async fn prune_drops_rows_below_the_snapshot_floor_and_keeps_the_newest() {
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

    // 四个快照、四版正文：facet 的 v 依次是 1、2、3、4。
    flush(&gateway, vec![ensure(), facet("v1")]).await;
    flush(&gateway, vec![ensure(), facet("v2")]).await;
    flush(&gateway, vec![ensure(), facet("v3")]).await;
    flush(&gateway, vec![ensure(), facet("v4")]).await;
    orphan_blob(&gateway).await;
    assert_eq!(gateway.store_counts().await.unwrap().snapshots, 4);
    assert_eq!(gateway.store_counts().await.unwrap().facets, 4);
    assert_eq!(gateway.store_counts().await.unwrap().blobs, 1);

    // 只裁掉最早一条快照：剩下的下界是 facets v2、resources v1。
    let snapshots = gateway.snapshots().await.unwrap();
    let cutoff = Timestamp::parse(&snapshots[1].published_at).unwrap();
    let plan = PrunePlan {
        snapshots_before: Some(cutoff),
        facet_keep: BTreeMap::from([("text".to_string(), 1)]),
        raw_before: Some(Timestamp::now()),
    };

    let preview = gateway.prune(&plan, true).await.unwrap();
    assert!(preview.dry_run);
    assert_eq!(preview.snapshots, 1);
    assert_eq!(preview.floor.get("facets"), Some(&2));
    // v1 因低于下界被删，v2、v3 因只保留最后一版被删。
    assert_eq!(preview.facets, 3);
    assert_eq!(preview.blobs, 1);
    // 试跑不写入。
    assert_eq!(gateway.store_counts().await.unwrap().snapshots, 4);
    assert_eq!(gateway.store_counts().await.unwrap().facets, 4);
    assert_eq!(gateway.store_counts().await.unwrap().blobs, 1);

    let report = gateway.prune(&plan, false).await.unwrap();
    assert_eq!(report.snapshots, 1);
    assert_eq!(report.facets, 3);
    assert_eq!(report.blobs, 1);

    let counts = gateway.store_counts().await.unwrap();
    assert_eq!(counts.snapshots, 3, "最新一条快照必须保留");
    assert_eq!(counts.facets, 1, "每 (rid, kind, variant) 只留最后一版");
    assert_eq!(counts.blobs, 0, "孤立 blob 行应当被清掉");
}

#[tokio::test]
async fn purge_resources_removes_rows_relations_and_objects() {
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
    let part = gateway.stage(&vec![9u8; 70_000]).await.unwrap();
    flush(
        &gateway,
        vec![
            ensure(),
            facet("保留正文"),
            Op::Ensure {
                identity: "ext:it:drop".into(),
                ns: None,
                labels: BTreeMap::new(),
                r#as: "drop".into(),
            },
            Op::PutFacet {
                target: Target::Alias("drop".into()),
                kind: FacetKind::Raw,
                variant: Some("binary".into()),
                body: Body::Staged { part },
                media_type: Some("application/octet-stream".into()),
                lineage: LineageInput {
                    inputs: Vec::new(),
                    params_hash: "purge".into(),
                },
            },
            Op::Link {
                src: Target::Alias("keep".into()),
                rel: "related".into(),
                dst: Target::Alias("drop".into()),
                attrs: None,
            },
        ],
    )
    .await;
    let view = gateway.view_latest().await.unwrap();
    let drop_rid = view
        .identities
        .iter()
        .find(|row| row.identity_key == "ext:it:drop")
        .unwrap()
        .rid
        .clone();
    let keep_rid = view
        .identities
        .iter()
        .find(|row| row.identity_key == "ext:it:keep")
        .unwrap()
        .rid
        .clone();
    assert_eq!(view.resources.len(), 2);
    assert_eq!(view.relations.len(), 1);
    assert_eq!(gateway.store_counts().await.unwrap().blobs, 1);

    let report = gateway
        .purge_resources(std::slice::from_ref(&drop_rid))
        .await
        .unwrap();
    assert_eq!(report.tables.get("g_resources"), Some(&1));
    assert_eq!(report.tables.get("g_relations"), Some(&1));
    assert_eq!(report.tables.get("g_identities"), Some(&1));
    assert_eq!(report.objects, 1);

    let after = gateway.view_latest().await.unwrap();
    assert_eq!(after.resources.len(), 1);
    assert_eq!(after.resources[0].rid, keep_rid);
    assert!(after.relations.is_empty(), "关系两侧都要清掉");
    assert!(after.identities.iter().all(|row| row.rid != drop_rid));
    assert_eq!(after.facets.len(), 1, "只保留另一资源的 facet");
    assert_eq!(gateway.store_counts().await.unwrap().blobs, 0);

    // 再次删除同一资源幂等，不再有行或对象。
    let again = gateway
        .purge_resources(std::slice::from_ref(&drop_rid))
        .await
        .unwrap();
    assert_eq!(again.tables.get("g_resources"), Some(&0));
    assert_eq!(again.objects, 0);
}

#[tokio::test]
async fn empty_plan_deletes_nothing() {
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
    flush(&gateway, vec![ensure(), facet("v1")]).await;

    // 缺省策略：三项都不清理。
    let report = gateway.prune(&PrunePlan::default(), false).await.unwrap();
    assert_eq!(report.snapshots, 0);
    assert_eq!(report.facets, 0);
    assert_eq!(report.blobs, 0);
    assert_eq!(report.resources, 0);
    assert_eq!(gateway.store_counts().await.unwrap().snapshots, 1);
    assert_eq!(gateway.store_counts().await.unwrap().facets, 1);
}
