//! 投影与热榜变更集。类型实现不调用 apply。

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use config::builtin_presets;
use serde_json::{Value, json};
use strata_model::{
    ApplyOpts, Body, ChangeSet, FacetKind, LineageInput, Op, Producer, Rid, Target,
};
use types::{Article, HotList, ReadRoot, invalid_input};

fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn config_dir(dir: &Path, hotlist: Option<&str>) {
    for preset in builtin_presets() {
        write(dir, &format!("presets/{}", preset.file_name), preset.source);
    }
    write(
        dir,
        "system.yaml",
        "id: system\nkind: system\nname: ModalPrism\npolicy:\n  preset: standard\nstrata:\n  gateway:\n    blob_backend: dir\n",
    );
    if let Some(body) = hotlist {
        write(dir, "types/hotlist.yaml", body);
    }
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn producer() -> Producer {
    Producer {
        module: "test".into(),
        version: "1".into(),
        config_generation: 1,
    }
}

fn lineage() -> LineageInput {
    LineageInput {
        inputs: Vec::new(),
        params_hash: "params".into(),
    }
}

fn ensure(identity: &str, alias: &str) -> Op {
    Op::Ensure {
        identity: identity.into(),
        ns: None,
        labels: BTreeMap::new(),
        r#as: alias.into(),
    }
}

async fn apply(root: &ReadRoot, ops: Vec<Op>) {
    root.strata()
        .apply(
            ChangeSet {
                producer: producer(),
                idempotency_key: None,
                ack: None,
                ops,
            },
            ApplyOpts::default(),
        )
        .await
        .unwrap();
    root.strata().flush().await.unwrap();
}

async fn resolve(root: &ReadRoot, identity: &str) -> String {
    root.strata()
        .resolve(identity)
        .await
        .unwrap()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn article_projects_without_body_bytes() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path(), None);
    let root = ReadRoot::open(data.path(), config.path()).await.unwrap();
    assert!(data.path().join("gateway.db").is_file());
    assert!(data.path().join("blobs").is_dir());
    assert!(data.path().join("staging").is_dir());
    assert_eq!(Article::NAME, "Article");
    assert!(Article::schema()["properties"]["cover"].is_object());
    assert_eq!(
        Article::selector().has_facets[0].variant.as_deref(),
        Some("article")
    );

    let cover = root.strata().stage(b"cover-bytes").await.unwrap();
    apply(
        &root,
        vec![
            ensure("ext:article:one", "article"),
            Op::PutFacet {
                target: Target::Alias("article".into()),
                kind: FacetKind::Struct,
                variant: Some("article".into()),
                body: Body::Struct {
                    value: json!({"title": "你好"}),
                },
                media_type: Some("application/json".into()),
                lineage: lineage(),
            },
            Op::PutFacet {
                target: Target::Alias("article".into()),
                kind: FacetKind::Md,
                variant: Some("readable".into()),
                body: Body::Text {
                    text: "正文".into(),
                },
                media_type: Some("text/markdown".into()),
                lineage: lineage(),
            },
            Op::PutFacet {
                target: Target::Alias("article".into()),
                kind: FacetKind::Image,
                variant: Some("cover".into()),
                body: Body::Staged {
                    part: cover.clone(),
                },
                media_type: Some("image/png".into()),
                lineage: lineage(),
            },
        ],
    )
    .await;
    let rid = resolve(&root, "ext:article:one").await;
    let projected = root
        .call("type_article.project", json!({ "rid": rid }))
        .await
        .unwrap();
    assert_eq!(projected["rid"], rid);
    assert_eq!(projected["struct"]["title"], "你好");
    assert_eq!(projected["readable"], "正文");
    assert_eq!(projected["cover"]["sha256"], cover.sha256());
    assert_eq!(projected["cover"]["size"], cover.size());
    assert!(projected.get("bytes").is_none());
    assert!(
        root.call("type_article.on_commit", json!({}))
            .await
            .unwrap()
            .is_null()
    );
    assert!(root.strata().flush().await.unwrap().is_none());

    let missing = root
        .call(
            "type_article.project",
            json!({ "rid": "r_01ARZ3NDEKTSV4RRFFQ69G5FAV" }),
        )
        .await
        .unwrap();
    assert!(missing.is_null());

    let facet = root
        .call(
            "strata_read.get",
            json!({ "facet": format!("{rid}@image:cover#latest") }),
        )
        .await
        .unwrap();
    assert!(facet.get("bytes").is_none());
    assert_eq!(facet["part"]["sha256"], cover.sha256());

    let health = root
        .call("health.status", json!({ "source": "demo" }))
        .await
        .unwrap();
    assert_eq!(health["key"], "source/demo");
    assert_eq!(health["status"], "ok");
    assert_eq!(health["alert"], false);
    apply(
        &root,
        vec![Op::SetState {
            scope: "health".into(),
            key: "source/demo".into(),
            value: json!({ "status": "degraded", "alert": true }),
            expect_version: None,
        }],
    )
    .await;
    let health = root
        .call("health.status", json!({ "source": "demo" }))
        .await
        .unwrap();
    assert_eq!(health["status"], "degraded");
    assert_eq!(health["alert"], true);

    let Err(error) = root.call("type_article.project", json!([])).await else {
        panic!("参数应当无效");
    };
    assert!(invalid_input(&error.message));
}

#[tokio::test]
async fn article_reads_text_stored_only_as_blob() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path(), None);
    let root = ReadRoot::open(data.path(), config.path()).await.unwrap();
    let text = "a".repeat(strata_client::INLINE_TEXT_MAX as usize + 1);
    apply(
        &root,
        vec![
            ensure("ext:article:blob", "article"),
            Op::PutFacet {
                target: Target::Alias("article".into()),
                kind: FacetKind::Struct,
                variant: Some("article".into()),
                body: Body::Struct {
                    value: json!({ "title": "blob" }),
                },
                media_type: Some("application/json".into()),
                lineage: lineage(),
            },
            Op::PutFacet {
                target: Target::Alias("article".into()),
                kind: FacetKind::Md,
                variant: Some("readable".into()),
                body: Body::Text { text: text.clone() },
                media_type: Some("text/markdown".into()),
                lineage: lineage(),
            },
        ],
    )
    .await;
    let rid = resolve(&root, "ext:article:blob").await;
    let lazy = root
        .call(
            "strata_read.get",
            json!({ "facet": format!("{rid}@md:readable#latest"), "lazy": true }),
        )
        .await
        .unwrap();
    assert!(lazy["text"].is_null());
    let projected = root
        .call("type_article.project", json!({ "rid": rid }))
        .await
        .unwrap();
    assert_eq!(projected["readable"], text);
}

#[tokio::test]
async fn hotlist_delta_is_a_changeset_and_keep_snapshots_tombstones() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(
        config.path(),
        Some("id: type/hotlist\nkind: type\nname: HotList\nspec:\n  keepSnapshots: 2\n"),
    );
    let root = ReadRoot::open(data.path(), config.path()).await.unwrap();
    assert_eq!(HotList::VERSION, "1");
    apply(
        &root,
        vec![
            ensure("ext:list:board", "list"),
            ensure("ext:item:a", "a"),
            ensure("ext:item:b", "b"),
            ensure("ext:item:c", "c"),
        ],
    )
    .await;
    let list = resolve(&root, "ext:list:board").await;
    let item_a = resolve(&root, "ext:item:a").await;
    let item_b = resolve(&root, "ext:item:b").await;
    let item_c = resolve(&root, "ext:item:c").await;

    apply(
        &root,
        snapshot("ext:snap:1", "s1", &list, &[(&item_a, 1), (&item_b, 2)]),
    )
    .await;
    let first = resolve(&root, "ext:snap:1").await;
    wait_past(&created_at(&root, &first).await).await;
    apply(
        &root,
        snapshot(
            "ext:snap:2",
            "s2",
            &list,
            &[(&item_a, 2), (&item_b, 1), (&item_c, 3)],
        ),
    )
    .await;
    let second = resolve(&root, "ext:snap:2").await;

    let changeset = root
        .call("type_hotlist.on_commit", json!({}))
        .await
        .unwrap();
    assert!(root.strata().flush().await.unwrap().is_none());
    assert_eq!(changeset["producer"]["module"], "type_hotlist");
    assert_eq!(changeset["producer"]["version"], "1");
    assert_eq!(changeset["producer"]["configGeneration"], 1);
    assert_eq!(changeset["idempotencyKey"].as_str().unwrap().len(), 64);
    let ops = changeset["ops"].as_array().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0]["op"], "putFacet");
    assert_eq!(ops[0]["variant"], "hotlist.delta");
    assert_eq!(ops[0]["target"]["rid"], second);
    assert_eq!(ops[0]["lineage"]["paramsHash"], "hotlist.delta");
    assert_eq!(ops[0]["mediaType"], "application/json");
    let mut expected_delta = vec![
        json!({ "dst": item_a, "from": 1, "to": 2, "delta": -1 }),
        json!({ "dst": item_b, "from": 2, "to": 1, "delta": 1 }),
    ];
    expected_delta.sort_by(|left, right| {
        left["dst"]
            .as_str()
            .unwrap()
            .cmp(right["dst"].as_str().unwrap())
    });
    assert_eq!(
        ops[0]["body"]["value"]["rankDelta"],
        Value::Array(expected_delta)
    );
    assert_eq!(
        ops[0]["body"]["value"]["newEntries"],
        json!([item_c.clone()])
    );

    let parsed: ChangeSet = serde_json::from_value(changeset).unwrap();
    root.strata()
        .apply(parsed, ApplyOpts::default())
        .await
        .unwrap();
    root.strata().flush().await.unwrap();
    let projected = root
        .call("type_hotlist.project", json!({ "rid": second }))
        .await
        .unwrap();
    assert_eq!(projected["list"], list);
    assert_eq!(projected["entries"][0]["dst"], item_b);
    assert_eq!(projected["entries"][0]["rank"], 1);
    assert_eq!(projected["delta"]["newEntries"][0], item_c);
    assert!(
        root.call("type_hotlist.on_commit", json!({}))
            .await
            .unwrap()
            .is_null()
    );

    wait_past(&created_at(&root, &second).await).await;
    apply(
        &root,
        snapshot(
            "ext:snap:3",
            "s3",
            &list,
            &[(&item_a, 2), (&item_b, 1), (&item_c, 3)],
        ),
    )
    .await;
    let trim = root
        .call("type_hotlist.on_commit", json!({}))
        .await
        .unwrap();
    assert!(root.strata().flush().await.unwrap().is_none());
    let trim_ops = trim["ops"].as_array().unwrap();
    assert!(trim_ops.iter().any(|op| {
        op["op"] == "tombstone" && op["reason"] == "keepSnapshots" && op["target"]["rid"] == first
    }));
    let parsed: ChangeSet = serde_json::from_value(trim).unwrap();
    root.strata()
        .apply(parsed, ApplyOpts::default())
        .await
        .unwrap();
    root.strata().flush().await.unwrap();
    let page = root
        .call(
            "strata_read.query",
            json!({
                "query": {
                    "from": "resources",
                    "hasFacets": [{ "kind": "struct", "variant": "hotlist.snapshot" }],
                    "includeTombstoned": true
                }
            }),
        )
        .await
        .unwrap();
    let rows = page["rows"].as_array().unwrap();
    let marked = rows.iter().find(|row| row["rid"] == first).unwrap();
    assert_eq!(marked["tombstone"], true);
}

async fn created_at(root: &ReadRoot, rid: &str) -> String {
    let page = root.strata().query(HotList::selector()).await.unwrap();
    page.rows
        .iter()
        .find(|row| row.get("rid").and_then(Value::as_str) == Some(rid))
        .unwrap()
        .get("createdAt")
        .and_then(Value::as_str)
        .unwrap()
        .to_string()
}

async fn wait_past(stamp: &str) {
    let parsed = strata_model::Timestamp::parse(stamp).unwrap();
    for _ in 0..100 {
        if strata_model::Timestamp::now() > parsed {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("时间没有前进");
}

fn snapshot(identity: &str, alias: &str, list: &str, entries: &[(&str, i64)]) -> Vec<Op> {
    let list = Rid::parse(list).unwrap();
    let mut ops = vec![
        ensure(identity, alias),
        Op::PutFacet {
            target: Target::Alias(alias.into()),
            kind: FacetKind::Struct,
            variant: Some("hotlist.snapshot".into()),
            body: Body::Struct {
                value: json!({ "period": identity }),
            },
            media_type: Some("application/json".into()),
            lineage: lineage(),
        },
        Op::Link {
            src: Target::Alias(alias.into()),
            rel: "part_of".into(),
            dst: Target::Rid(list),
            attrs: None,
        },
    ];
    for (dst, rank) in entries {
        ops.push(Op::Link {
            src: Target::Alias(alias.into()),
            rel: "entry".into(),
            dst: Target::Rid(Rid::parse(dst).unwrap()),
            attrs: Some(json!({ "rank": rank })),
        });
    }
    ops
}

#[tokio::test]
async fn postgres_without_url_is_refused_before_the_data_dir() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path(), None);
    write(
        config.path(),
        "system.yaml",
        "id: system\nkind: system\nname: ModalPrism\npolicy:\n  preset: standard\nstrata:\n  gateway:\n    backend: postgres\n",
    );
    let Err(error) = ReadRoot::open(data.path(), config.path()).await else {
        panic!("缺少连接串的 PostgreSQL 应当被拒绝");
    };
    assert_eq!(error.to_string(), "存储位置未规定");
    assert!(invalid_input(&error.message));
    assert!(!data.path().join("gateway.db").exists());
    assert!(!data.path().join("blobs").exists());
    assert!(!data.path().join("staging").exists());
}

#[tokio::test]
async fn memory_blob_backend_keeps_metadata_local() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path(), None);
    write(
        config.path(),
        "system.yaml",
        "id: system\nkind: system\nname: ModalPrism\npolicy:\n  preset: standard\nstrata:\n  gateway:\n    blob_backend: memory\n",
    );
    let root = ReadRoot::open(data.path(), config.path()).await.unwrap();
    assert!(
        root.strata()
            .resolve("ext:doi:missing")
            .await
            .unwrap()
            .is_none()
    );
    drop(root);
    assert!(data.path().join("gateway.db").is_file());
    assert!(!data.path().join("blobs").exists());
    assert!(data.path().join("staging").is_dir());
}

#[tokio::test]
async fn workspace_config_loads_type_modules() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path(), None);
    // 模块取自仓库里的真实配置，数据目录用临时后端：这条用例不依赖二进制与网络。
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../config");
    for name in ["types", "schemas"] {
        copy_dir(&workspace.join(name), &config.path().join(name));
    }
    let root = ReadRoot::open(data.path(), config.path()).await.unwrap();
    let snapshot = root.config().snapshot();
    assert!(snapshot.modules.contains_key("type/article"));
    assert_eq!(
        snapshot.modules["type/hotlist"]["spec"]["keepSnapshots"],
        288
    );
    assert!(HotList::schema()["properties"]["entries"].is_object());
}

#[tokio::test]
async fn read_root_uses_configured_blob_thresholds() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path(), None);
    write(
        config.path(),
        "system.yaml",
        "id: system\nkind: system\nname: ModalPrism\npolicy:\n  preset: standard\nstrata:\n  blob:\n    inline_size_threshold: 8KiB\n    dedicated_size_threshold: 1KiB\n  staging:\n    ttl: 12h\n",
    );
    let Err(error) = ReadRoot::open(data.path(), config.path()).await else {
        panic!("倒挂的 blob 阈值应当拒绝打开");
    };
    assert!(error.to_string().starts_with("blob 阈值无效"), "{error}");

    write(
        config.path(),
        "system.yaml",
        "id: system\nkind: system\nname: ModalPrism\npolicy:\n  preset: standard\nstrata:\n  blob:\n    inline_size_threshold: 32KiB\n    dedicated_size_threshold: 8MiB\n  staging:\n    ttl: 12h\n",
    );
    let opened = tempfile::tempdir().unwrap();
    ReadRoot::open(opened.path(), config.path()).await.unwrap();
}
