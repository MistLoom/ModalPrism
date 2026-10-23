use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use serde_json::json;
use strata_client::{Options, Strata};
use strata_model::{
    AccessTask, ApplyOpts, AsOf, Body, ChangeSet, EdgeDir, Expr, FacetKind, FacetRef, LineageInput,
    Locator, Op, OrderBy, Producer, Query, QueryFrom, QueueName, Search, SelectItem, SourceId,
    SpanRef, Target, TaskId, TaskReason, Timestamp, VersionSel, WaitFor,
};

fn producer() -> Producer {
    Producer {
        module: "client-test".into(),
        version: "1".into(),
        config_generation: 4,
    }
}

fn lineage() -> LineageInput {
    LineageInput {
        inputs: Vec::new(),
        params_hash: "params".into(),
    }
}

fn ensure(identity: &str, alias: &str, labels: BTreeMap<String, String>) -> Op {
    Op::Ensure {
        identity: identity.into(),
        ns: None,
        labels,
        r#as: alias.into(),
    }
}

fn text(alias: &str, text: &str) -> Op {
    Op::PutFacet {
        target: Target::Alias(alias.into()),
        kind: FacetKind::Text,
        variant: None,
        body: Body::Text { text: text.into() },
        media_type: Some("text/plain".into()),
        lineage: lineage(),
    }
}

fn changeset(ops: Vec<Op>) -> ChangeSet {
    ChangeSet {
        producer: producer(),
        idempotency_key: None,
        ack: None,
        ops,
    }
}

#[tokio::test]
async fn storage_roundtrip_matches_phase_exit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let strata = Strata::open(root, Options::default()).await.unwrap();
    assert!(root.join("gateway.db").is_file());
    assert!(root.join("blobs").is_dir());
    assert!(root.join("staging").is_dir());

    let mut labels = BTreeMap::new();
    labels.insert("lang".into(), "zh".into());
    let accepted = strata
        .apply(
            changeset(vec![
                ensure("ext:doi:10.1000/a", "a", labels.clone()),
                text("a", "one"),
            ]),
            ApplyOpts::default(),
        )
        .await
        .unwrap();
    assert_eq!(accepted.status, WaitFor::Accepted);
    assert!(accepted.snapshot.is_none());
    assert!(strata.resolve("ext:doi:10.1000/a").await.unwrap().is_none());

    let seq = strata.flush().await.unwrap().unwrap();
    assert_eq!(seq, 1);
    let rid = strata.resolve("ext:doi:10.1000/a").await.unwrap().unwrap();
    drop(strata);
    let again = Strata::open(root, Options::default()).await.unwrap();
    assert_eq!(
        again.resolve("ext:doi:10.1000/a").await.unwrap().unwrap(),
        rid
    );

    let mut repeated = changeset(vec![
        ensure("ext:doi:10.1000/a", "a", BTreeMap::new()),
        text("a", "one"),
    ]);
    repeated.idempotency_key = Some("same".into());
    let first = again
        .apply(repeated.clone(), ApplyOpts::default())
        .await
        .unwrap();
    let second = again.apply(repeated, ApplyOpts::default()).await.unwrap();
    assert!(second.duplicate);
    assert_eq!(first.commit_id, second.commit_id);
    again.flush().await.unwrap();
    let history = again
        .history(&FacetRef::new(rid.clone(), FacetKind::Text, None, VersionSel::Latest).unwrap())
        .await
        .unwrap();
    assert_eq!(history.len(), 1);

    again
        .apply(
            changeset(vec![
                ensure("ext:doi:10.1000/a", "a", BTreeMap::new()),
                text("a", "two"),
            ]),
            ApplyOpts::default(),
        )
        .await
        .unwrap();
    again.flush().await.unwrap();
    let history = again
        .history(&FacetRef::new(rid.clone(), FacetKind::Text, None, VersionSel::Latest).unwrap())
        .await
        .unwrap();
    assert_eq!(
        history.iter().map(|item| item.version).collect::<Vec<_>>(),
        vec![1, 2]
    );

    again
        .apply(
            changeset(vec![Op::Tombstone {
                target: Target::Rid(rid.clone()),
                reason: "gone".into(),
            }]),
            ApplyOpts::default(),
        )
        .await
        .unwrap();
    again.flush().await.unwrap();
    again
        .apply(
            changeset(vec![ensure("ext:doi:10.1000/a", "a", BTreeMap::new())]),
            ApplyOpts::default(),
        )
        .await
        .unwrap();
    again.flush().await.unwrap();
    assert_eq!(
        again.resolve("ext:doi:10.1000/a").await.unwrap().unwrap(),
        rid
    );

    let cursor = again
        .apply(
            changeset(vec![
                ensure("ext:doi:10.1000/b", "b", BTreeMap::new()),
                text("b", "body"),
                Op::SetState {
                    scope: "cursor".into(),
                    key: "src".into(),
                    value: json!("a"),
                    expect_version: Some(0),
                },
                Op::SetState {
                    scope: "receipt".into(),
                    key: "pay".into(),
                    value: json!({"ok": true}),
                    expect_version: None,
                },
            ]),
            ApplyOpts::default(),
        )
        .await
        .unwrap();
    let failed = again
        .apply(
            changeset(vec![Op::SetState {
                scope: "cursor".into(),
                key: "src".into(),
                value: json!("nope"),
                expect_version: Some(0),
            }]),
            ApplyOpts::default(),
        )
        .await;
    assert!(failed.is_err());
    again.flush().await.unwrap();
    let state = again.state("cursor", "src").await.unwrap().unwrap();
    assert_eq!(state.value, json!("a"));
    assert_eq!(state.version, 1);
    assert!(
        again.state("receipt", "pay").await.unwrap().unwrap().value["ok"]
            .as_bool()
            .unwrap()
    );
    let moved = again
        .apply(
            changeset(vec![Op::SetState {
                scope: "cursor".into(),
                key: "src".into(),
                value: json!("b"),
                expect_version: Some(1),
            }]),
            ApplyOpts::default(),
        )
        .await
        .unwrap();
    assert_ne!(moved.commit_id, cursor.commit_id);
    again.flush().await.unwrap();
    assert_eq!(
        again.state("cursor", "src").await.unwrap().unwrap().version,
        2
    );

    let task = AccessTask {
        task_id: TaskId::new("task-1").unwrap(),
        source_id: SourceId::new("src").unwrap(),
        accessor: "http".into(),
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
    again.enqueue(std::slice::from_ref(&task)).await.unwrap();
    let leased = again
        .lease(QueueName::Free, 1, Duration::from_secs(30))
        .await
        .unwrap();
    assert_eq!(leased.len(), 1);
    assert!(
        again
            .lease(QueueName::Free, 1, Duration::from_secs(30))
            .await
            .unwrap()
            .is_empty()
    );
    again
        .extend(&task.task_id, Duration::from_secs(30))
        .await
        .unwrap();
    again
        .nack(
            &task.task_id,
            Timestamp::parse("2000-01-01T00:00:00Z").unwrap(),
            "later",
        )
        .await
        .unwrap();
    let retried = again
        .lease(QueueName::Free, 1, Duration::from_secs(30))
        .await
        .unwrap();
    assert_eq!(retried[0].attempt, 1);
    again
        .apply(
            ChangeSet {
                producer: producer(),
                idempotency_key: None,
                ack: Some(task.task_id.clone()),
                ops: vec![ensure("ext:doi:10.1000/c", "c", BTreeMap::new())],
            },
            ApplyOpts::default(),
        )
        .await
        .unwrap();
    assert!(
        again
            .lease(QueueName::Free, 1, Duration::from_secs(30))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        again
            .extend(&task.task_id, Duration::from_secs(5))
            .await
            .is_err()
    );

    let page = again
        .query(Query {
            from: QueryFrom::Resources,
            r#where: Some(
                Expr::try_from(json!({"all": [{"eq": ["ns", "data"]}, {"gte": ["seq", 1]}]}))
                    .unwrap(),
            ),
            has_facets: Vec::new(),
            select: vec![
                SelectItem::Field("rid".into()),
                SelectItem::Field("tombstone".into()),
            ],
            expand: Vec::new(),
            search: None,
            as_of: None,
            order_by: Vec::new(),
            limit: Some(1),
            cursor: None,
        })
        .await
        .unwrap();
    assert_eq!(page.rows.len(), 1);
    assert!(page.next_cursor.is_some());
    let search = again
        .query(Query {
            from: QueryFrom::Resources,
            r#where: None,
            has_facets: Vec::new(),
            select: Vec::new(),
            expand: Vec::new(),
            search: Some(Search {
                text: Some("two".into()),
                vector: None,
                variant: None,
                k: 5,
            }),
            as_of: None,
            order_by: Vec::new(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    assert!(
        search
            .rows
            .iter()
            .any(|row| row["rid"].as_str() == Some(rid.as_str())),
        "{search:?}"
    );
    assert!(search.next_cursor.is_none());
    let unknown = again
        .query(Query {
            from: QueryFrom::Resources,
            r#where: Some(Expr::try_from(json!({"nope": true})).unwrap()),
            has_facets: Vec::new(),
            select: Vec::new(),
            expand: Vec::new(),
            search: None,
            as_of: None,
            order_by: Vec::new(),
            limit: None,
            cursor: None,
        })
        .await;
    assert!(unknown.is_err());
    let ordered = again
        .query(Query {
            from: QueryFrom::Facets,
            r#where: None,
            has_facets: Vec::new(),
            select: Vec::new(),
            expand: Vec::new(),
            search: None,
            as_of: None,
            order_by: vec![OrderBy {
                field: "rid".into(),
                descending: false,
            }],
            limit: None,
            cursor: Some("1".into()),
        })
        .await;
    assert!(ordered.is_err());
    let tagged = again
        .query(Query {
            from: QueryFrom::Resources,
            r#where: None,
            has_facets: Vec::new(),
            select: Vec::new(),
            expand: Vec::new(),
            search: None,
            as_of: Some(AsOf::Tag("audit".into())),
            order_by: Vec::new(),
            limit: None,
            cursor: None,
        })
        .await;
    assert!(tagged.is_err());

    let embedding = again
        .apply(
            changeset(vec![Op::PutFacet {
                target: Target::Rid(rid.clone()),
                kind: FacetKind::Embedding,
                variant: Some("m".into()),
                body: Body::Text { text: "v".into() },
                media_type: None,
                lineage: lineage(),
            }]),
            ApplyOpts::default(),
        )
        .await;
    assert!(embedding.is_err());
    let external = again
        .apply(
            changeset(vec![Op::PutFacet {
                target: Target::Rid(rid.clone()),
                kind: FacetKind::File,
                variant: None,
                body: Body::External {
                    uri: "s3://bucket/a".into(),
                },
                media_type: None,
                lineage: lineage(),
            }]),
            ApplyOpts::default(),
        )
        .await;
    assert!(external.is_err());

    let part = again.stage(b"raw-bytes").await.unwrap();
    let staged_path = root.join("staging").join(part.sha256());
    assert!(staged_path.is_file());
    let visible = again
        .apply(
            ChangeSet {
                producer: producer(),
                idempotency_key: None,
                ack: None,
                ops: vec![
                    ensure("ext:doi:10.1000/file", "f", BTreeMap::new()),
                    Op::PutFacet {
                        target: Target::Alias("f".into()),
                        kind: FacetKind::File,
                        variant: None,
                        body: Body::Staged { part: part.clone() },
                        media_type: Some("application/octet-stream".into()),
                        lineage: lineage(),
                    },
                ],
            },
            ApplyOpts {
                wait: WaitFor::Visible,
                timeout: Some(Duration::from_secs(30)),
            },
        )
        .await
        .unwrap();
    assert_eq!(visible.status, WaitFor::Visible);
    assert!(!staged_path.exists());
    let file_rid = again
        .resolve("ext:doi:10.1000/file")
        .await
        .unwrap()
        .unwrap();
    let loaded = again
        .get(
            &FacetRef::new(file_rid.clone(), FacetKind::File, None, VersionSel::Latest).unwrap(),
            false,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.bytes.unwrap().as_ref(), b"raw-bytes");
    let lazy = again
        .get(
            &FacetRef::new(file_rid.clone(), FacetKind::File, None, VersionSel::Latest).unwrap(),
            true,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(lazy.bytes.is_none());
    assert_eq!(lazy.blob.unwrap().sha256(), part.sha256());
    let slice = again
        .slice(
            &SpanRef::new(
                FacetRef::new(file_rid, FacetKind::File, None, VersionSel::Latest).unwrap(),
                0,
                3,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(slice.as_ref(), b"raw");
    drop(again);

    let orphan = "ab".repeat(32);
    std::fs::write(root.join("staging").join(&orphan), b"old").unwrap();
    let old = SystemTime::now() - Duration::from_secs(48 * 60 * 60);
    filetime_set(root.join("staging").join(&orphan), old);
    let short = Strata::open(
        root,
        Options {
            staging_ttl: Duration::from_secs(1),
            ..Options::default()
        },
    )
    .await
    .unwrap();
    short
        .apply(
            changeset(vec![ensure("ext:doi:10.1000/sweep", "s", BTreeMap::new())]),
            ApplyOpts::default(),
        )
        .await
        .unwrap();
    short.flush().await.unwrap();
    assert!(!root.join("staging").join(&orphan).exists());

    let past = short
        .query(Query {
            from: QueryFrom::Resources,
            r#where: None,
            has_facets: Vec::new(),
            select: Vec::new(),
            expand: Vec::new(),
            search: None,
            as_of: Some(AsOf::Time(
                Timestamp::parse("2000-01-01T00:00:00Z").unwrap(),
            )),
            order_by: Vec::new(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    assert!(past.rows.is_empty());
    let changes = short.changes(0, Some(2)).await.unwrap();
    assert_eq!(changes.len(), 2);
    assert!(
        short
            .related(&rid, None, EdgeDir::Out)
            .await
            .unwrap()
            .is_empty()
    );
}

fn search_query(
    text: &str,
    variant: Option<&str>,
    as_of: Option<AsOf>,
    select: Vec<SelectItem>,
) -> Query {
    Query {
        from: QueryFrom::Resources,
        r#where: None,
        has_facets: Vec::new(),
        select,
        expand: Vec::new(),
        search: Some(Search {
            text: Some(text.into()),
            vector: None,
            variant: variant.map(str::to_string),
            k: 5,
        }),
        as_of,
        order_by: Vec::new(),
        limit: None,
        cursor: None,
    }
}

#[tokio::test]
async fn search_reads_the_snapshot_and_skips_stale_versions() {
    let dir = tempfile::tempdir().unwrap();
    let strata = Strata::open(dir.path(), Options::default()).await.unwrap();
    strata
        .apply(
            changeset(vec![
                ensure("ext:doi:search/a", "a", BTreeMap::new()),
                Op::PutFacet {
                    target: Target::Alias("a".into()),
                    kind: FacetKind::Text,
                    variant: Some("body".into()),
                    body: Body::Text {
                        text: "alpha one".into(),
                    },
                    media_type: None,
                    lineage: lineage(),
                },
            ]),
            ApplyOpts::default(),
        )
        .await
        .unwrap();
    let first = strata.flush().await.unwrap().unwrap();
    let rid = strata.resolve("ext:doi:search/a").await.unwrap().unwrap();

    strata
        .apply(
            changeset(vec![
                ensure("ext:doi:search/b", "b", BTreeMap::new()),
                Op::PutFacet {
                    target: Target::Alias("b".into()),
                    kind: FacetKind::Text,
                    variant: Some("body".into()),
                    body: Body::Text {
                        text: "beta two".into(),
                    },
                    media_type: None,
                    lineage: lineage(),
                },
            ]),
            ApplyOpts::default(),
        )
        .await
        .unwrap();
    strata.flush().await.unwrap();
    let other = strata.resolve("ext:doi:search/b").await.unwrap().unwrap();

    let page = strata
        .query(search_query("two", None, None, Vec::new()))
        .await
        .unwrap();
    assert_eq!(page.rows.len(), 1, "{page:?}");
    assert_eq!(page.rows[0]["rid"], json!(other));
    assert!(page.next_cursor.is_none());

    // 旧版本正文被替换后不再命中。
    strata
        .apply(
            changeset(vec![Op::PutFacet {
                target: Target::Rid(rid.clone()),
                kind: FacetKind::Text,
                variant: Some("body".into()),
                body: Body::Text {
                    text: "gamma".into(),
                },
                media_type: None,
                lineage: lineage(),
            }]),
            ApplyOpts::default(),
        )
        .await
        .unwrap();
    strata.flush().await.unwrap();
    let stale = strata
        .query(search_query("alpha", Some("body"), None, Vec::new()))
        .await
        .unwrap();
    assert!(stale.rows.is_empty(), "{stale:?}");
    let fresh = strata
        .query(search_query(
            "gamma",
            Some("body"),
            None,
            vec![SelectItem::Field("rid".into())],
        ))
        .await
        .unwrap();
    assert_eq!(fresh.rows.len(), 1, "{fresh:?}");
    assert_eq!(fresh.rows[0].as_object().unwrap().len(), 1, "{fresh:?}");
    let missing = strata
        .query(search_query("gamma", Some("title"), None, Vec::new()))
        .await
        .unwrap();
    assert!(missing.rows.is_empty(), "{missing:?}");

    let past = strata
        .query(search_query(
            "two",
            None,
            Some(AsOf::Snapshot(first)),
            Vec::new(),
        ))
        .await
        .unwrap();
    assert!(past.rows.is_empty(), "{past:?}");
    let earlier = strata
        .query(search_query(
            "alpha",
            None,
            Some(AsOf::Snapshot(first)),
            Vec::new(),
        ))
        .await
        .unwrap();
    assert!(
        earlier.rows.iter().any(|row| row["rid"] == json!(rid)),
        "{earlier:?}"
    );
}

fn filetime_set(path: std::path::PathBuf, time: SystemTime) {
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(time).unwrap();
}

#[test]
fn facade_source_has_no_table_api() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let sources = ["Cargo.toml", "src/lib.rs", "src/query.rs", "src/options.rs"];
    for name in sources {
        let text = std::fs::read_to_string(root.join(name)).unwrap();
        assert!(!text.contains("lancedb"), "{name}");
        assert!(!text.contains("lance"), "{name}");
        assert!(!text.contains("datafusion"), "{name}");
        assert!(!text.contains("sqlx"), "{name}");
    }
}
