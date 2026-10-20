use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use bytes::Bytes;
use serde_json::{Value, json};
use strata_model::{
    Body, FacetKind, Locator, Op, PartId, PartRole, Producer, RenderLevel, Rid, Target, TaskReason,
    Timestamp,
};

use adapters::{
    AdapterError, AdapterInput, AdapterMeta, AdapterRole, ContentAdapter, ContentOutput, Detection,
    Detector, DetectorChoice, DiscoverOutput, DraftBody, FacetDraft, Follow, IdentityHit,
    InputSelector, MapIdentities, OutputDecl, PartBytes, Probe, Quality, ReadonlyView, Registry,
    RunRequest, SearchAdapter, empty_identities, parse_pipeline, run, source_view,
};

struct CountDetector {
    inner: adapters::BasicDetector,
    hits: Arc<AtomicU32>,
}

#[async_trait]
impl Detector for CountDetector {
    fn id(&self) -> &str {
        "detector/basic"
    }

    fn version(&self) -> &str {
        "1"
    }

    async fn detect(
        &self,
        view: &ReadonlyView,
        declared: &adapters::Declared,
    ) -> Result<Detection, AdapterError> {
        self.hits.fetch_add(1, Ordering::Relaxed);
        self.inner.detect(view, declared).await
    }

    async fn regions(&self, view: &ReadonlyView, wanted: &[String]) -> Vec<adapters::RegionHit> {
        self.inner.regions(view, wanted).await
    }
}

struct ProbeAdapter {
    accept: bool,
}

#[async_trait]
impl SearchAdapter for ProbeAdapter {
    fn meta(&self) -> &AdapterMeta {
        static META: std::sync::OnceLock<AdapterMeta> = std::sync::OnceLock::new();
        META.get_or_init(|| AdapterMeta {
            id: "adapter/probe".into(),
            version: "1".into(),
            role: AdapterRole::Search,
            consumes: vec![InputSelector {
                kind: None,
                variant: None,
                part_role: Some("document".into()),
                media_type: None,
            }],
            produces: vec![OutputDecl::Discovery],
            deterministic: true,
        })
    }

    async fn probe(&self, _input: &AdapterInput, _spec: &Value) -> Option<Probe> {
        Some(Probe {
            accept: self.accept,
            confidence: 1.0,
        })
    }

    async fn discover(
        &self,
        input: &AdapterInput,
        _spec: &Value,
    ) -> Result<DiscoverOutput, AdapterError> {
        Ok(DiscoverOutput {
            items: vec![adapters::Discovery {
                locator: input.locator.clone(),
                identity: vec!["url:https://example.com/probed".into()],
                hints: Default::default(),
                span: None,
                follow: Follow::No,
                materialize: Vec::new(),
            }],
            feedback: None,
        })
    }
}

struct MarkAdapter;

#[async_trait]
impl ContentAdapter for MarkAdapter {
    fn meta(&self) -> &AdapterMeta {
        static META: std::sync::OnceLock<AdapterMeta> = std::sync::OnceLock::new();
        META.get_or_init(|| AdapterMeta {
            id: "adapter/mark".into(),
            version: "1".into(),
            role: AdapterRole::Content,
            consumes: vec![InputSelector {
                kind: Some("md".into()),
                variant: None,
                part_role: None,
                media_type: None,
            }],
            produces: vec![OutputDecl::Facet {
                kind: "text".into(),
                variant: None,
            }],
            deterministic: true,
        })
    }

    async fn adapt(
        &self,
        input: &AdapterInput,
        _spec: &Value,
    ) -> Result<ContentOutput, AdapterError> {
        let text = String::from_utf8_lossy(&input.view.bytes);
        Ok(ContentOutput {
            facets: vec![FacetDraft {
                kind: FacetKind::Text,
                variant: None,
                body: DraftBody::Text(format!("MARK\n{text}")),
                media_type: Some("text/plain".into()),
                derived_from: Vec::new(),
            }],
            links: Vec::new(),
            feedback: None,
        })
    }
}

struct RandomAdapter;

#[async_trait]
impl ContentAdapter for RandomAdapter {
    fn meta(&self) -> &AdapterMeta {
        static META: std::sync::OnceLock<AdapterMeta> = std::sync::OnceLock::new();
        META.get_or_init(|| AdapterMeta {
            id: "adapter/random".into(),
            version: "1".into(),
            role: AdapterRole::Content,
            consumes: Vec::new(),
            produces: vec![OutputDecl::Facet {
                kind: "text".into(),
                variant: None,
            }],
            deterministic: false,
        })
    }

    async fn adapt(
        &self,
        _input: &AdapterInput,
        _spec: &Value,
    ) -> Result<ContentOutput, AdapterError> {
        Ok(ContentOutput {
            facets: Vec::new(),
            links: Vec::new(),
            feedback: None,
        })
    }
}

fn producer() -> Producer {
    Producer {
        module: "runner".into(),
        version: "1".into(),
        config_generation: 1,
    }
}

fn request(parts: Vec<PartBytes>, pipeline: Value, reason: TaskReason) -> RunRequest {
    RunRequest {
        parts,
        pipeline: parse_pipeline(&pipeline).unwrap(),
        reason,
        source: source_view("source/demo", "accessor/http").unwrap(),
        resource: Some(Rid::parse("r_01JAF3K8Z2X7Q9M4T6V0B5N1CE").unwrap()),
        origin: None,
        locator: Locator::Url {
            url: "https://example.com/blog/post".into(),
        },
        detector: DetectorChoice::Basic,
        admission: Default::default(),
        now: Timestamp::parse("2026-10-20T00:00:00Z").unwrap(),
        initial: false,
        producer: producer(),
        specs: Default::default(),
        identities: empty_identities(),
    }
}

fn part(media: &str, body: &str) -> PartBytes {
    PartBytes {
        part_id: PartId::new("p1").unwrap(),
        role: PartRole::Document,
        declared: media.into(),
        charset: Some("utf-8".into()),
        bytes: Bytes::from(body.to_string()),
        facet: None,
    }
}

fn registry_with_counter(hits: Arc<AtomicU32>) -> Registry {
    let mut registry = Registry::builtins();
    registry.register_detector(Arc::new(CountDetector {
        inner: adapters::BasicDetector,
        hits,
    }));
    registry
}

fn text_of(op: &Op) -> Option<&str> {
    match op {
        Op::PutFacet {
            body: Body::Text { text },
            ..
        } => Some(text),
        _ => None,
    }
}

#[tokio::test]
async fn declared_json_skips_detection_and_is_deterministic() {
    let hits = Arc::new(AtomicU32::new(0));
    let registry = registry_with_counter(Arc::clone(&hits));
    let pipeline = json!([{
        "when": {"part_role": "document", "declared": ["application/json"]},
        "run": ["adapter/json-list"],
        "spec": {"adapter/json-list": {"items": "/items", "url": "url", "title": "title"}}
    }]);
    let body = r#"{"items":[{"url":"https://example.com/a","title":"A"},{"url":"https://example.com/b","title":"B"}]}"#;
    let first = run(
        &registry,
        request(
            vec![part("application/json", body)],
            pipeline.clone(),
            TaskReason::Schedule,
        ),
    )
    .await
    .unwrap();
    let second = run(
        &registry,
        request(
            vec![part("application/json", body)],
            pipeline,
            TaskReason::Schedule,
        ),
    )
    .await
    .unwrap();
    assert_eq!(hits.load(Ordering::Relaxed), 0);
    assert_eq!(first.changeset, second.changeset);
    assert_eq!(first.follows.len(), 2);
    assert!(
        first
            .follows
            .iter()
            .all(|item| item.accessor == "accessor/http")
    );
    let titles: Vec<_> = first
        .changeset
        .ops
        .iter()
        .filter_map(|op| match op {
            Op::PutFacet {
                kind: FacetKind::Struct,
                variant: Some(variant),
                body: Body::Struct { value },
                ..
            } if variant == "discovery" => value.pointer("/hints/title").and_then(Value::as_str),
            _ => None,
        })
        .collect();
    assert_eq!(titles, ["A", "B"]);
}

#[tokio::test]
async fn detection_runs_once_and_is_shared() {
    let hits = Arc::new(AtomicU32::new(0));
    let registry = registry_with_counter(Arc::clone(&hits));
    let html = r#"<html><head><title>Hello page</title>
        <script type="application/ld+json">{"@type":"Article","name":"Hello"}</script>
        </head><body><article><h1>Hello</h1><p>Para <a href="/a">link</a> and enough words to pass the thin content threshold for the readability adapter.</p><img src="/i.png"></article></body></html>"#;
    let pipeline = json!([{
        "when": {"part_role": "document", "detected": ["text/html"]},
        "run": ["adapter/readability-md", "adapter/jsonld-struct", "adapter/link-discovery"]
    }]);
    let output = run(
        &registry,
        request(
            vec![part("application/octet-stream", html)],
            pipeline,
            TaskReason::Schedule,
        ),
    )
    .await
    .unwrap();
    assert_eq!(hits.load(Ordering::Relaxed), 1);
    let detect = output.changeset.ops.iter().find_map(|op| match op {
        Op::PutFacet {
            kind: FacetKind::Struct,
            variant: Some(variant),
            body: Body::Struct { value },
            ..
        } if variant == "detect" => Some(value),
        _ => None,
    });
    let detect = detect.unwrap();
    assert_eq!(detect["mediaType"], "text/html");
    assert!(
        detect["traits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item == "has-jsonld")
    );
    assert!(
        detect["traits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item == "has-images")
    );
    assert!(output.changeset.ops.iter().any(|op| matches!(
        op,
        Op::PutFacet { kind: FacetKind::Regions, variant: Some(variant), .. } if variant == "detect"
    )));
    let md = output.changeset.ops.iter().find_map(|op| match op {
        Op::PutFacet {
            kind: FacetKind::Md,
            variant: Some(variant),
            body: Body::Text { text },
            ..
        } if variant == "readable" => Some(text.as_str()),
        _ => None,
    });
    let md = md.unwrap();
    assert!(md.contains("# Hello"), "{md}");
    assert!(md.contains("[link](https://example.com/a)"), "{md}");
    assert!(!md.contains("nav"));
    assert!(output.changeset.ops.iter().any(|op| matches!(
        op,
        Op::PutFacet { kind: FacetKind::Struct, variant: Some(variant), .. } if variant == "jsonld"
    )));
    assert!(output.follows.iter().any(|item| item.locator
        == Locator::Url {
            url: "https://example.com/a".into()
        }));
}

#[tokio::test]
async fn probe_gates_run_if_accepts() {
    let mut registry = Registry::builtins();
    registry
        .register_search(Arc::new(ProbeAdapter { accept: false }))
        .unwrap();
    let pipeline = json!([{
        "when": {"part_role": "document"},
        "run_if_accepts": ["adapter/probe"]
    }]);
    let denied = run(
        &registry,
        request(
            vec![part("text/plain", "hello")],
            pipeline.clone(),
            TaskReason::Schedule,
        ),
    )
    .await
    .unwrap();
    assert!(denied.changeset.ops.is_empty());
    registry
        .register_search(Arc::new(ProbeAdapter { accept: true }))
        .unwrap();
    let accepted = run(
        &registry,
        request(
            vec![part("text/plain", "hello")],
            pipeline,
            TaskReason::Schedule,
        ),
    )
    .await
    .unwrap();
    assert!(
        accepted
            .changeset
            .ops
            .iter()
            .any(|op| matches!(op, Op::Ensure { .. }))
    );
}

#[tokio::test]
async fn after_uses_upstream_markdown() {
    let mut registry = Registry::builtins();
    registry.register_content(Arc::new(MarkAdapter)).unwrap();
    let html = r#"<html><body><nav>nav</nav><article><h1>Hello</h1><p>Para <a href="/a">link</a> and enough words to pass the thin content threshold for the readability adapter.</p></article></body></html>"#;
    let pipeline = json!([
        {"when": {"part_role": "document", "detected": ["text/html"]}, "run": ["adapter/readability-md"]},
        {"when": {"part_role": "document", "detected": ["text/html"]}, "after": "adapter/readability-md", "run": ["adapter/mark"]}
    ]);
    let output = run(
        &registry,
        request(
            vec![part("text/html", html)],
            pipeline,
            TaskReason::Schedule,
        ),
    )
    .await
    .unwrap();
    let marked = output
        .changeset
        .ops
        .iter()
        .filter_map(text_of)
        .find(|text| text.starts_with("MARK\n"));
    let marked = marked.unwrap();
    assert!(marked.contains("# Hello"), "{marked}");
    assert!(!marked.contains("nav"), "{marked}");
}

#[tokio::test]
async fn follow_rules_do_not_run_on_schedule() {
    let registry = Registry::builtins();
    let html = r#"<html><body><article><h1>Hello</h1><p>Para <a href="/a">link</a> and enough words to pass the thin content threshold for the readability adapter.</p></article></body></html>"#;
    let pipeline = json!([
        {"when": {"part_role": "document", "detected": ["text/html"]}, "run": ["adapter/html-list"], "spec": {"adapter/html-list": {"item": "article", "link": "a", "title": "h1"}}},
        {"for": "follow", "when": {"part_role": "document", "detected": ["text/html"]}, "run": ["adapter/readability-md"]}
    ]);
    let scheduled = run(
        &registry,
        request(
            vec![part("text/html", html)],
            pipeline.clone(),
            TaskReason::Schedule,
        ),
    )
    .await
    .unwrap();
    assert!(
        scheduled
            .changeset
            .ops
            .iter()
            .any(|op| matches!(op, Op::Ensure { .. }))
    );
    assert!(!scheduled.changeset.ops.iter().any(|op| matches!(
        op,
        Op::PutFacet {
            kind: FacetKind::Md,
            ..
        }
    )));
    let followed = run(
        &registry,
        request(vec![part("text/html", html)], pipeline, TaskReason::Follow),
    )
    .await
    .unwrap();
    assert!(followed.changeset.ops.iter().any(|op| matches!(
        op,
        Op::PutFacet {
            kind: FacetKind::Md,
            ..
        }
    )));
    assert!(
        !followed
            .changeset
            .ops
            .iter()
            .any(|op| matches!(op, Op::Ensure { .. }))
    );
}

#[tokio::test]
async fn feed_materialize_admission_and_tombstone() {
    let registry = Registry::builtins();
    let feed = r#"<?xml version="1.0"?>
        <rss version="2.0"><channel><title>Example</title>
        <item><title>One</title><link>https://example.com/1</link><guid>item-1</guid>
        <pubDate>Tue, 20 Oct 2026 00:00:00 +0000</pubDate><description>Summary</description>
        <content:encoded xmlns:content="http://purl.org/rss/1.0/modules/content/"><![CDATA[<p>Full</p>]]></content:encoded></item>
        <item><title>Sponsor deal</title><link>https://example.com/ad</link><description>buy</description></item>
        <item><title>Two</title><link>https://example.com/2</link><pubDate>Tue, 01 Jan 2020 00:00:00 +0000</pubDate></item>
        <item><title>Three</title><link>https://other.test/3</link></item>
        </channel></rss>"#;
    let pipeline = json!([{"run": ["adapter/feed-items", "adapter/feed-fulltext"]}]);
    let mut identities = MapIdentities::default();
    identities.hits.insert(
        "url:https://example.com/2".into(),
        IdentityHit::Tombstone(Rid::parse("r_01JAF3K8Z2X7Q9M4T6V0B5N1CE").unwrap()),
    );
    let mut req = request(
        vec![part("application/rss+xml", feed)],
        pipeline,
        TaskReason::Schedule,
    );
    req.initial = true;
    req.admission = adapters::Admission {
        allow_prefixes: vec!["https://example.com/".into()],
        deny_prefixes: Vec::new(),
        published_after: None,
        backfill_limit: Some(1),
        backfill_max_age: Some(std::time::Duration::from_secs(86_400 * 365 * 10)),
        noise: vec!["sponsor".into()],
    };
    req.identities = Arc::new(identities);
    let output = run(&registry, req).await.unwrap();
    assert_eq!(output.follows.len(), 0);
    let ensures: Vec<_> = output
        .changeset
        .ops
        .iter()
        .filter_map(|op| match op {
            Op::Ensure { identity, r#as, .. } => Some((identity.as_str(), r#as.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(ensures, [("url:https://example.com/1", "d0")]);
    assert!(output.changeset.ops.iter().any(|op| matches!(
        op,
        Op::PutFacet { kind: FacetKind::Html, target: Target::Alias(alias), body: Body::Text { text }, .. }
            if alias == "d0" && text.contains("Full")
    )));
    assert!(
        output
            .changeset
            .ops
            .iter()
            .any(|op| text_of(op).is_some_and(|text| text.contains("# Example")))
    );
}

#[tokio::test]
async fn existing_identity_does_not_follow_or_materialize() {
    let registry = Registry::builtins();
    let feed = r#"<?xml version="1.0"?><rss><channel><item><title>One</title><link>https://example.com/1</link><content:encoded><![CDATA[<p>Full</p>]]></content:encoded></item></channel></rss>"#;
    let pipeline = json!([{"run": ["adapter/feed-items"]}]);
    let mut identities = MapIdentities::default();
    let rid = Rid::parse("r_01JAF3K8Z2X7Q9M4T6V0B5N1CE").unwrap();
    identities.hits.insert(
        "url:https://example.com/1".into(),
        IdentityHit::Live(rid.clone()),
    );
    let mut req = request(
        vec![part("application/rss+xml", feed)],
        pipeline,
        TaskReason::Schedule,
    );
    req.identities = Arc::new(identities);
    let output = run(&registry, req).await.unwrap();
    assert!(output.follows.is_empty());
    assert!(
        !output
            .changeset
            .ops
            .iter()
            .any(|op| matches!(op, Op::Ensure { .. }))
    );
    assert!(output.changeset.ops.iter().any(|op| matches!(
        op,
        Op::Link { rel, src: Target::Rid(src), .. } if rel == "discovered_by" && src == &rid
    )));
    assert!(!output.changeset.ops.iter().any(|op| matches!(
        op,
        Op::PutFacet {
            kind: FacetKind::Html,
            ..
        }
    )));
}

#[tokio::test]
async fn shell_page_asks_for_render_and_cycle_fails() {
    let registry = Registry::builtins();
    let pipeline = json!([{
        "when": {"detected": ["text/html"]},
        "run": ["adapter/readability-md"]
    }]);
    let output = run(
        &registry,
        request(
            vec![part("text/html", "<html><body>hi</body></html>")],
            pipeline,
            TaskReason::Schedule,
        ),
    )
    .await
    .unwrap();
    assert_eq!(output.feedback.len(), 1);
    assert_eq!(output.feedback[0].feedback.quality, Quality::Blocked);
    assert_eq!(
        output.feedback[0].feedback.needs_render,
        Some(RenderLevel::L2)
    );

    let mut registry = Registry::builtins();
    registry.register_content(Arc::new(MarkAdapter)).unwrap();
    let pipeline = json!([{"run": ["adapter/mark"], "after": "adapter/mark"}]);
    let error = run(
        &registry,
        request(
            vec![part("text/plain", "hello")],
            pipeline,
            TaskReason::Schedule,
        ),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("环"), "{error}");
}

#[tokio::test]
async fn nondeterministic_adapter_is_rejected() {
    let mut registry = Registry::builtins();
    registry.register_content(Arc::new(RandomAdapter)).unwrap();
    let pipeline = json!([{"run": ["adapter/random"]}]);
    let error = run(
        &registry,
        request(
            vec![part("text/plain", "hello")],
            pipeline,
            TaskReason::Schedule,
        ),
    )
    .await
    .unwrap_err();
    assert_eq!(error.class(), strata_model::ErrorClass::Config);
    assert!(error.to_string().contains("确定性"));
}
