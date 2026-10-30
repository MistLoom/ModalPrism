//! 来源归属：内容身份用 url/ext 去重，来源由 `source` 标签落成 `resources.source_id`。
//!
//! 内容库按"可见信源"过滤时读的是该列，因此标签缺失会让整个内容库为空。

use std::collections::BTreeMap;

use serde_json::json;
use strata_gateway::{BlobBackend, Gateway, Options, Request};
use strata_model::{
    ApplyOpts, Body, ChangeSet, Expr, FacetKind, LineageInput, Op, Producer, QueryFrom, Target,
    WaitFor,
};

fn changeset(ops: Vec<Op>) -> ChangeSet {
    ChangeSet {
        producer: Producer {
            module: "attribution-it".into(),
            version: "1".into(),
            config_generation: 1,
        },
        idempotency_key: None,
        ack: None,
        ops,
    }
}

fn ensure(alias: &str, identity: &str, source: Option<&str>) -> Op {
    let mut labels = BTreeMap::new();
    if let Some(source) = source {
        labels.insert("source".to_string(), source.to_string());
    }
    Op::Ensure {
        identity: identity.into(),
        ns: None,
        labels,
        r#as: alias.into(),
    }
}

fn struct_facet(alias: &str) -> Op {
    Op::PutFacet {
        target: Target::Alias(alias.into()),
        kind: FacetKind::Struct,
        variant: Some("article".into()),
        body: Body::Struct {
            value: json!({"title": "条目"}),
        },
        media_type: Some("application/json".into()),
        lineage: LineageInput {
            inputs: Vec::new(),
            params_hash: "seed".into(),
        },
    }
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
    (dir, gateway)
}

async fn count(gateway: &Gateway, source: &str) -> usize {
    let mut request = Request::new(QueryFrom::Resources);
    request.filter =
        Some(serde_json::from_value::<Expr>(json!({"eq": ["sourceId", source]})).unwrap());
    gateway.query(request).await.unwrap().rows.len()
}

#[tokio::test]
async fn source_label_lands_in_the_source_id_column() {
    let (_dir, gateway) = open().await;
    let ops = vec![
        ensure("a", "url:https://example.com/a", Some("demo-feed")),
        struct_facet("a"),
        ensure("b", "url:https://example.com/b", None),
        struct_facet("b"),
    ];
    let accepted = gateway
        .apply(changeset(ops), ApplyOpts::default())
        .await
        .unwrap();
    assert_eq!(accepted.status, WaitFor::Accepted);
    gateway.flush().await.unwrap();

    assert_eq!(count(&gateway, "demo-feed").await, 1);
    let all = gateway
        .query(Request::new(QueryFrom::Resources))
        .await
        .unwrap();
    assert_eq!(all.rows.len(), 2, "未带来源标签的行仍然可见");
}

/// 身份键自带 `src:` 时以身份为准，标签不覆盖显式来源。
#[tokio::test]
async fn src_identity_wins_over_label() {
    let (_dir, gateway) = open().await;
    let ops = vec![
        ensure("a", "src:push-feed:abc", Some("other")),
        struct_facet("a"),
    ];
    gateway
        .apply(changeset(ops), ApplyOpts::default())
        .await
        .unwrap();
    gateway.flush().await.unwrap();

    assert_eq!(count(&gateway, "push-feed").await, 1);
    assert_eq!(count(&gateway, "other").await, 0);
}
