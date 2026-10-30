use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{Value, json};
use strata_model::{
    AccessTask, ApplyOpts, AsOf, Body, ChangeSet, EdgeDir, ErrorClass, Expand, Expr, FacetKind,
    FacetRef, IdentityKey, LineageInput, Locator, ModelError, Namespace, Op, PartRef, PartRole,
    Producer, Query, QueryFrom, QueueName, RawEnvelope, RawPart, RenderLevel, Rid, SelectItem,
    Signals, SpanRef, Target, TaskReason, Timestamp, Transport, VersionSel, WaitFor, content_hash,
};

#[test]
fn rid_is_prefixed_ulid_and_canonical() {
    let generated = Rid::generate();
    let text = generated.as_str();
    assert!(text.starts_with("r_"));
    assert_eq!(text.len(), 28);
    assert_eq!(Rid::parse(text).unwrap().as_str(), text);

    let parsed = Rid::parse("r_01jaf3k8z2x7q9m4t6v0b5n1ce").unwrap();
    assert_eq!(parsed.as_str(), "r_01JAF3K8Z2X7Q9M4T6V0B5N1CE");
    assert!(Rid::parse("01JAF3K8Z2X7Q9M4T6V0B5N1CE").is_err());
    assert!(Rid::parse("r_01JAF3K8Z2X7Q9M4T6V0B5N1C").is_err());
    assert!(Rid::parse("r_01JAF3K8Z2X7Q9M4T6V0B5N1CI").is_err());
}

#[test]
fn generated_rids_differ() {
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..64 {
        assert!(seen.insert(Rid::generate().as_str().to_string()));
    }
}

#[test]
fn facet_and_span_refs_roundtrip() {
    let facet = FacetRef::parse("r_01JAF3K8Z2X7Q9M4T6V0B5N1CE@md:readable#v3").unwrap();
    assert_eq!(facet.kind, FacetKind::Md);
    assert_eq!(facet.variant.as_deref(), Some("readable"));
    assert_eq!(facet.version, VersionSel::Exact(3));
    assert_eq!(
        facet.to_string(),
        "r_01JAF3K8Z2X7Q9M4T6V0B5N1CE@md:readable#v3"
    );

    let latest = FacetRef::parse("r_01JAF3K8Z2X7Q9M4T6V0B5N1CE@html#latest").unwrap();
    assert_eq!(latest.version, VersionSel::Latest);
    assert_eq!(
        latest.to_string(),
        "r_01JAF3K8Z2X7Q9M4T6V0B5N1CE@html#latest"
    );

    let span = SpanRef::parse("r_01JAF3K8Z2X7Q9M4T6V0B5N1CE@raw#v1[1024..2048]").unwrap();
    assert_eq!(span.start, 1024);
    assert_eq!(span.end, 2048);
    assert_eq!(
        span.to_string(),
        "r_01JAF3K8Z2X7Q9M4T6V0B5N1CE@raw#v1[1024..2048]"
    );
    assert!(SpanRef::parse("r_01JAF3K8Z2X7Q9M4T6V0B5N1CE@raw#v1[2048..1024]").is_err());
    assert!(FacetRef::parse("r_01JAF3K8Z2X7Q9M4T6V0B5N1CE@struct#v1").is_err());
    assert!(FacetRef::parse("r_01JAF3K8Z2X7Q9M4T6V0B5N1CE@embedding:bge-m3#v1").is_ok());
}

#[test]
fn content_hash_and_part_ref() {
    let bytes = b"abc";
    assert_eq!(
        content_hash(bytes),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let part = PartRef::from_bytes(bytes);
    assert_eq!(part.size(), 3);
    assert_eq!(part.sha256(), content_hash(bytes));

    let parsed = PartRef::new(part.sha256().to_ascii_uppercase(), 3).unwrap();
    assert_eq!(parsed, part);
    assert!(PartRef::new("abcd", 1).is_err());

    let value = serde_json::to_value(&part).unwrap();
    assert_eq!(value["sha256"], part.sha256());
    assert_eq!(serde_json::from_value::<PartRef>(value).unwrap(), part);
}

#[test]
fn namespace_and_identity_keys() {
    assert_eq!(Namespace::parse("data").unwrap(), Namespace::Data);
    assert_eq!(Namespace::parse("sys").unwrap(), Namespace::Sys);
    assert_eq!(
        Namespace::parse("type:article").unwrap().to_string(),
        "type:article"
    );
    assert!(Namespace::parse("type:").is_err());

    let external = IdentityKey::parse("ext:doi:10.1000/182").unwrap();
    assert_eq!(external.to_string(), "ext:doi:10.1000/182");
    let url = IdentityKey::parse("url:https://example.com/a").unwrap();
    assert_eq!(url.to_string(), "url:https://example.com/a");
    let source = IdentityKey::parse("src:source/example-blog:abc").unwrap();
    assert_eq!(source.to_string(), "src:source/example-blog:abc");
    assert!(IdentityKey::parse("title:hello").is_err());
}

#[test]
fn unclassified_error_is_transient() {
    assert_eq!(ErrorClass::resolve(None), ErrorClass::Transient);
    assert_eq!(
        ErrorClass::resolve(Some(ErrorClass::Budget)),
        ErrorClass::Budget
    );
    let value = serde_json::to_value(ErrorClass::Config).unwrap();
    assert_eq!(value, json!("config"));
}

#[test]
fn changeset_serde_and_validation() {
    let span = SpanRef::parse("r_01JAF3K8Z2X7Q9M4T6V0B5N1CE@raw#v1[0..4]").unwrap();
    let cs = ChangeSet {
        producer: Producer {
            module: "adapter/readability-md".into(),
            version: "1.4.0".into(),
            config_generation: 37,
        },
        idempotency_key: Some("k1".into()),
        ack: None,
        ops: vec![
            Op::Ensure {
                identity: "url:https://example.com/a".into(),
                ns: Some("data".into()),
                labels: BTreeMap::new(),
                r#as: "a".into(),
            },
            Op::PutFacet {
                target: Target::Alias("a".into()),
                kind: FacetKind::Md,
                variant: Some("readable".into()),
                body: Body::Text {
                    text: "hello".into(),
                },
                media_type: Some("text/markdown".into()),
                lineage: LineageInput {
                    inputs: vec![span],
                    params_hash: "9f3c".into(),
                },
            },
        ],
    };
    cs.validate().unwrap();
    let value = serde_json::to_value(&cs).unwrap();
    assert_eq!(value["producer"]["configGeneration"], 37);
    assert_eq!(value["idempotencyKey"], "k1");
    assert!(value.get("ack").is_none());
    assert_eq!(value["ops"][0]["op"], "ensure");
    assert_eq!(value["ops"][0]["as"], "a");
    assert_eq!(value["ops"][1]["op"], "putFacet");
    assert_eq!(value["ops"][1]["kind"], "md");
    assert_eq!(value["ops"][1]["mediaType"], "text/markdown");
    assert!(value["ops"][1].get("media_type").is_none());
    assert_eq!(value["ops"][1]["body"]["type"], "text");
    let state: Op = serde_json::from_value(json!({
        "op": "setState",
        "scope": "cursor",
        "key": "source/a",
        "value": 1,
        "expectVersion": 3
    }))
    .unwrap();
    match state {
        Op::SetState { expect_version, .. } => assert_eq!(expect_version, Some(3)),
        other => panic!("期望 setState，得到 {other:?}"),
    }
    assert!(
        serde_json::from_value::<Op>(json!({
            "op": "setState",
            "scope": "cursor",
            "key": "source/a",
            "value": 1,
            "expect_version": 3
        }))
        .is_err()
    );
    let back: ChangeSet = serde_json::from_value(value).unwrap();
    assert_eq!(back, cs);

    let mut missing = cs.clone();
    if let Op::PutFacet { kind, variant, .. } = &mut missing.ops[1] {
        *kind = FacetKind::Struct;
        *variant = None;
    }
    assert_eq!(missing.validate(), Err(ModelError::MissingVariant));
}

#[test]
fn query_rejects_string_expression() {
    let err = serde_json::from_value::<Query>(json!({
        "from": "resources",
        "where": "ns = 'data'"
    }));
    assert!(err.is_err());

    let query: Query = serde_json::from_value(json!({
        "from": "resources",
        "where": { "all": [ { "eq": ["ns", "data"] } ] },
        "hasFacets": [ { "kind": "struct", "variant": "article" }, { "kind": "md", "variant": "readable" } ],
        "select": [ "rid", { "kind": "struct", "variant": "article", "fields": ["title"] } ],
        "expand": [ { "rel": "discovered_by", "dir": "out" } ],
        "limit": 50
    }))
    .unwrap();
    assert_eq!(query.from, QueryFrom::Resources);
    assert!(query.r#where.is_some());
    assert_eq!(query.has_facets.len(), 2);
    assert!(matches!(query.select[0], SelectItem::Field(_)));
    assert!(matches!(query.select[1], SelectItem::Facet(_)));
    assert_eq!(query.expand[0].dir, EdgeDir::Out);
    assert_eq!(query.expand[0].rel, "discovered_by");
    query.validate().unwrap();

    let mut zero = query.clone();
    zero.limit = Some(0);
    assert_eq!(zero.validate(), Err(ModelError::InvalidQuery));

    let expr = Expr::try_from(json!({"eq": ["ns", "data"]})).unwrap();
    assert!(expr.as_map().contains_key("eq"));
    assert!(Expr::try_from(Value::String("x".into())).is_err());
}

#[test]
fn envelope_carries_part_ref_not_bytes() {
    let started = Timestamp::parse("2026-10-18T00:00:00+08:00").unwrap();
    let envelope = RawEnvelope {
        envelope_id: "env-1".parse_id(),
        locator: Locator::Url {
            url: "https://example.com/a".into(),
        },
        final_locator: None,
        transport: Transport {
            protocol: "https".into(),
            status: Some(200),
            headers: vec![("content-type".into(), "text/html".into())],
            started_at: started,
            duration: Duration::from_secs(2),
            bytes_in: 4,
            not_modified: false,
            render_level: Some(RenderLevel::L2),
            engine: None,
            receipt_id: None,
        },
        parts: vec![RawPart {
            part_id: "part-1".parse_id(),
            role: PartRole::Document,
            media_type: "text/html".into(),
            charset: Some("utf-8".into()),
            locator: None,
            bytes: PartRef::from_bytes(b"html"),
            meta: serde_json::Map::new(),
        }],
        cursor_candidate: None,
        signals: Signals {
            retry_after: Some(Duration::from_secs(30)),
            ttl: None,
            cache_max_age: None,
        },
    };
    envelope.validate().unwrap();
    let value = serde_json::to_value(&envelope).unwrap();
    assert!(
        value["parts"][0]
            .get("bytes")
            .unwrap()
            .get("sha256")
            .is_some()
    );
    assert!(value["parts"][0].get("body").is_none());
    assert_eq!(value["parts"][0]["role"], "document");
    assert_eq!(value["transport"]["renderLevel"], "L2");
    assert_eq!(value["transport"]["duration"], "2s");
    assert_eq!(value["signals"]["retryAfter"], "30s");
    let back: RawEnvelope = serde_json::from_value(value).unwrap();
    assert_eq!(back, envelope);
}

#[test]
fn access_task_roundtrip() {
    let task = AccessTask {
        task_id: "task-1".parse_id(),
        source_id: "source/example-blog".parse_id(),
        plan_id: None,
        accessor: "accessor/http".into(),
        locator: Locator::Url {
            url: "https://example.com/blog/".into(),
        },
        spec: json!({}),
        reason: TaskReason::Schedule,
        priority: 10,
        queue: QueueName::Free,
        lease_until: Timestamp::parse("2026-10-18T01:00:00Z").unwrap(),
        attempt: 1,
        batch_key: None,
        generation: 4,
    };
    task.validate().unwrap();
    let value = serde_json::to_value(&task).unwrap();
    assert_eq!(value["sourceId"], "source/example-blog");
    assert_eq!(value["reason"], "schedule");
    assert_eq!(value["queue"], "free");
    assert_eq!(value["leaseUntil"], "2026-10-18T01:00:00Z");
    let back: AccessTask = serde_json::from_value(value).unwrap();
    assert_eq!(back, task);
}

#[test]
fn apply_opts_default_to_accepted() {
    let opts = ApplyOpts::default();
    assert_eq!(opts.wait, WaitFor::Accepted);
    assert!(opts.timeout.is_none());
    let value = serde_json::to_value(&opts).unwrap();
    assert_eq!(value["wait"], "accepted");
}

#[test]
fn as_of_shapes() {
    let at = Timestamp::parse("2026-10-18T00:00:00Z").unwrap();
    let snapshot = serde_json::to_value(AsOf::Snapshot(7)).unwrap();
    let time = serde_json::to_value(AsOf::Time(at)).unwrap();
    let tag = serde_json::to_value(AsOf::Tag("daily".into())).unwrap();
    assert_eq!(snapshot, json!({"snapshot": 7}));
    assert_eq!(time["time"], "2026-10-18T00:00:00Z");
    assert_eq!(tag, json!({"tag": "daily"}));
    assert_eq!(
        serde_json::from_value::<AsOf>(snapshot).unwrap(),
        AsOf::Snapshot(7)
    );
}

trait ParseId<T> {
    fn parse_id(self) -> T;
}

impl ParseId<strata_model::EnvelopeId> for &str {
    fn parse_id(self) -> strata_model::EnvelopeId {
        strata_model::EnvelopeId::new(self).unwrap()
    }
}

impl ParseId<strata_model::PartId> for &str {
    fn parse_id(self) -> strata_model::PartId {
        strata_model::PartId::new(self).unwrap()
    }
}

impl ParseId<strata_model::TaskId> for &str {
    fn parse_id(self) -> strata_model::TaskId {
        strata_model::TaskId::new(self).unwrap()
    }
}

impl ParseId<strata_model::SourceId> for &str {
    fn parse_id(self) -> strata_model::SourceId {
        strata_model::SourceId::new(self).unwrap()
    }
}

#[test]
fn expand_is_constructed() {
    let expand = Expand {
        rel: "links_to".into(),
        dir: EdgeDir::In,
        depth: Some(1),
    };
    assert_eq!(expand.depth, Some(1));
}
