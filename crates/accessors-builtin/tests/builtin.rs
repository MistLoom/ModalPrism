use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::Bytes;
use flate2::Compression;
use flate2::write::GzEncoder;
use serde_json::json;
use strata_model::{ErrorClass, Locator, PartRole, SourceId, TaskId, Timestamp};
use tokio::sync::Mutex;

use accessors::{
    AccessContext, AccessHints, AccessRequest, Accessor, EgressGateway, HttpRequest, HttpResponse,
    PaidCall, PaidGateway, PaidResponse, Session, SessionPool, Staging,
};
use accessors_builtin::{
    BrowserAccessor, BrowserEngine, EngineOutput, EngineRequest, Incoming, MatchedBody,
    MoliCommand, Notice, PushAccessor, by_id, moli_args, notify, subscribe, verify,
};

struct ScriptEgress {
    hits: AtomicUsize,
    responses: Mutex<Vec<HttpResponse>>,
    seen: Mutex<Vec<HttpRequest>>,
}

#[async_trait]
impl EgressGateway for ScriptEgress {
    async fn request(&self, request: HttpRequest) -> Result<HttpResponse, accessors::AccessError> {
        self.hits.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().await.push(request);
        let mut responses = self.responses.lock().await;
        if responses.is_empty() {
            return Ok(HttpResponse {
                status: 200,
                headers: Vec::new(),
                body: Bytes::new(),
                final_url: "https://example.com/final".into(),
            });
        }
        Ok(responses.remove(0))
    }
}

struct ScriptPaid {
    hits: AtomicUsize,
    fail_budget: bool,
}

#[async_trait]
impl PaidGateway for ScriptPaid {
    async fn call(&self, _request: PaidCall) -> Result<PaidResponse, accessors::AccessError> {
        let hit = self.hits.fetch_add(1, Ordering::SeqCst);
        if self.fail_budget {
            return Err(accessors::AccessError::new(ErrorClass::Budget, "预算不足"));
        }
        Ok(PaidResponse {
            status: 200,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: Bytes::from(format!(r#"{{"hit":{hit}}}"#)),
            receipt_id: strata_model::ReceiptId::new(format!("rcpt_{hit}")).ok(),
        })
    }
}

struct ScriptSessions;

#[async_trait]
impl SessionPool for ScriptSessions {
    async fn get(&self, profile: &str) -> Result<Session, accessors::AccessError> {
        Ok(Session {
            headers: vec![("X-Profile".into(), profile.into())],
            cookie: Some("sid=1".into()),
        })
    }
}

struct MemStaging {
    puts: Arc<AtomicUsize>,
}

impl Staging for MemStaging {
    fn put(&self, bytes: &[u8]) -> Result<strata_model::PartRef, accessors::AccessError> {
        self.puts.fetch_add(1, Ordering::SeqCst);
        Ok(strata_model::PartRef::from_bytes(bytes))
    }
}

struct ScriptEngine {
    hits: AtomicUsize,
    output: EngineOutput,
}

#[async_trait]
impl BrowserEngine for ScriptEngine {
    async fn fetch(&self, request: &EngineRequest) -> Result<EngineOutput, accessors::AccessError> {
        self.hits.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request.proxy, "http://127.0.0.1:9");
        assert!(request.block_private_networks);
        Ok(self.output.clone())
    }
}

fn ctx(egress: Arc<ScriptEgress>, paid: Arc<ScriptPaid>) -> (AccessContext, Arc<AtomicUsize>) {
    let puts = Arc::new(AtomicUsize::new(0));
    (
        AccessContext {
            egress,
            paid,
            sessions: Arc::new(ScriptSessions),
            staging: Arc::new(MemStaging { puts: puts.clone() }),
            cancel: Arc::new(AtomicBool::new(false)),
        },
        puts,
    )
}

fn request(locator: Locator, spec: serde_json::Value) -> AccessRequest {
    AccessRequest {
        task_id: TaskId::new("task_1").unwrap(),
        source_id: SourceId::new("source/demo").unwrap(),
        locator,
        spec,
        cursor: None,
        hints: AccessHints::default(),
        policy: Arc::new(json!({"raw": {"redact_headers": ["Set-Cookie"]}})),
        deadline: Instant::now() + Duration::from_secs(5),
    }
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

#[tokio::test]
async fn http_decodes_gzip_redacts_and_skips_not_modified() {
    let body = gzip(b"<p>hello</p>");
    let egress = Arc::new(ScriptEgress {
        hits: AtomicUsize::new(0),
        responses: Mutex::new(vec![
            HttpResponse {
                status: 200,
                headers: vec![
                    ("Content-Type".into(), "text/html; charset=utf-8".into()),
                    ("Content-Encoding".into(), "gzip".into()),
                    ("Set-Cookie".into(), "secret".into()),
                    ("Cache-Control".into(), "max-age=30".into()),
                ],
                body: Bytes::from(body),
                final_url: "https://example.com/page".into(),
            },
            HttpResponse {
                status: 304,
                headers: vec![("ETag".into(), "abc".into())],
                body: Bytes::from_static(b"stale"),
                final_url: "https://example.com/page".into(),
            },
        ]),
        seen: Mutex::new(Vec::new()),
    });
    let (context, _puts) = ctx(
        egress.clone(),
        Arc::new(ScriptPaid {
            hits: AtomicUsize::new(0),
            fail_budget: false,
        }),
    );
    let accessor = by_id("http").unwrap();
    let mut req = request(
        Locator::Url {
            url: "https://example.com/page".into(),
        },
        json!({"session": "direct"}),
    );
    req.hints.etag = Some("abc".into());
    let page = accessor.access(req, &context).await.unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].parts.len(), 1);
    assert_eq!(page[0].parts[0].role, PartRole::Document);
    assert_eq!(page[0].parts[0].media_type, "text/html");
    assert_eq!(page[0].parts[0].charset.as_deref(), Some("utf-8"));
    assert_eq!(page[0].parts[0].bytes.size(), b"<p>hello</p>".len() as u64);
    assert_eq!(
        page[0]
            .transport
            .headers
            .iter()
            .find(|(name, _)| name == "Set-Cookie")
            .unwrap()
            .1,
        "***"
    );
    assert!(
        page[0]
            .transport
            .headers
            .iter()
            .all(|(name, _)| !name.eq_ignore_ascii_case("content-encoding"))
    );
    assert_eq!(page[0].signals.cache_max_age, Some(Duration::from_secs(30)));
    assert_eq!(
        page[0].transport.render_level,
        Some(strata_model::RenderLevel::L1)
    );
    let seen = egress.seen.lock().await;
    assert!(
        seen[0]
            .headers
            .iter()
            .any(|(name, value)| name == "If-None-Match" && value == "abc")
    );
    assert!(
        seen[0]
            .headers
            .iter()
            .any(|(name, value)| name == "Cookie" && value == "sid=1")
    );
    drop(seen);

    let mut again = request(
        Locator::Url {
            url: "https://example.com/page".into(),
        },
        json!({}),
    );
    again.hints.etag = Some("abc".into());
    let not_modified = accessor.access(again, &context).await.unwrap();
    assert!(not_modified[0].transport.not_modified);
    assert!(not_modified[0].parts.is_empty());
}

#[tokio::test]
async fn rss_follows_next_without_splitting_items() {
    let feed = r#"<rss><channel><item><title>One</title></item><link rel="next" href="https://example.com/feed?page=2"/></channel></rss>"#;
    let egress = Arc::new(ScriptEgress {
        hits: AtomicUsize::new(0),
        responses: Mutex::new(vec![
            HttpResponse {
                status: 200,
                headers: vec![("Content-Type".into(), "application/rss+xml".into())],
                body: Bytes::from(feed),
                final_url: "https://example.com/feed".into(),
            },
            HttpResponse {
                status: 200,
                headers: vec![(
                    "Link".into(),
                    "<https://example.com/feed?page=3>; rel=\"next\"".into(),
                )],
                body: Bytes::from_static(b"<rss><channel></channel></rss>"),
                final_url: "https://example.com/feed?page=2".into(),
            },
            HttpResponse {
                status: 200,
                headers: Vec::new(),
                body: Bytes::from_static(b"<rss/>"),
                final_url: "https://example.com/feed?page=3".into(),
            },
        ]),
        seen: Mutex::new(Vec::new()),
    });
    let (context, _puts) = ctx(
        egress.clone(),
        Arc::new(ScriptPaid {
            hits: AtomicUsize::new(0),
            fail_budget: false,
        }),
    );
    let accessor = by_id("rss").unwrap();
    let pages = accessor
        .access(
            request(
                Locator::Url {
                    url: "https://example.com/feed".into(),
                },
                json!({"pages": 2}),
            ),
            &context,
        )
        .await
        .unwrap();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].parts.len(), 1);
    assert_eq!(pages[0].parts[0].bytes.size(), feed.len() as u64);
    assert_eq!(egress.hits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn api_pages_by_offset_cursor_and_link() {
    let egress = Arc::new(ScriptEgress {
        hits: AtomicUsize::new(0),
        responses: Mutex::new(vec![
            HttpResponse {
                status: 200,
                headers: vec![("Content-Type".into(), "application/json".into())],
                body: Bytes::from_static(br#"{"next":"tok"}"#),
                final_url: "https://example.com/items?page=0".into(),
            },
            HttpResponse {
                status: 200,
                headers: vec![("Content-Type".into(), "application/json".into())],
                body: Bytes::from_static(br#"{"items":[]}"#),
                final_url: "https://example.com/items?page=10".into(),
            },
        ]),
        seen: Mutex::new(Vec::new()),
    });
    let (context, _puts) = ctx(
        egress.clone(),
        Arc::new(ScriptPaid {
            hits: AtomicUsize::new(0),
            fail_budget: false,
        }),
    );
    let accessor = by_id("api").unwrap();
    let pages = accessor
        .access(
            request(
                Locator::ApiEndpoint { endpoint: "https://example.com/items".into() },
                json!({"pages": 2, "pagination": {"type": "offset", "param": "page", "start": 0, "step": 10}}),
            ),
            &context,
        )
        .await
        .unwrap();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].cursor_candidate.as_ref().unwrap()["offset"], 10);
    let seen = egress.seen.lock().await;
    assert!(seen[0].url.contains("page=0"));
    assert!(seen[1].url.contains("page=10"));
    drop(seen);

    egress.responses.lock().await.clear();
    egress.responses.lock().await.push(HttpResponse {
        status: 200,
        headers: vec![("Content-Type".into(), "application/json".into())],
        body: Bytes::from_static(br#"{"next":"tok"}"#),
        final_url: "https://example.com/items".into(),
    });
    egress.responses.lock().await.push(HttpResponse {
        status: 500,
        headers: Vec::new(),
        body: Bytes::from_static(b"nope"),
        final_url: "https://example.com/items?cursor=tok".into(),
    });
    egress.seen.lock().await.clear();
    let pages = accessor
        .access(
            request(
                Locator::ApiEndpoint { endpoint: "https://example.com/items".into() },
                json!({"pages": 3, "pagination": {"type": "cursor", "param": "cursor", "pointer": "/next"}}),
            ),
            &context,
        )
        .await
        .unwrap();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].cursor_candidate.as_ref().unwrap()["token"], "tok");
    assert_eq!(egress.seen.lock().await.len(), 2);
}

#[tokio::test]
async fn paid_call_keeps_one_receipt_and_budget_error() {
    let paid = Arc::new(ScriptPaid {
        hits: AtomicUsize::new(0),
        fail_budget: false,
    });
    let (context, _puts) = ctx(
        Arc::new(ScriptEgress {
            hits: AtomicUsize::new(0),
            responses: Mutex::new(Vec::new()),
            seen: Mutex::new(Vec::new()),
        }),
        paid.clone(),
    );
    let accessor = by_id("paid-api").unwrap();
    assert!(accessor.capabilities().batchable);
    assert_eq!(
        accessor.capabilities().paid.as_ref().unwrap().service,
        "spec"
    );
    let pages = accessor
        .access(
            request(
                Locator::ApiEndpoint {
                    endpoint: "/v1/search".into(),
                },
                json!({"service": "search", "params": {"q": "a"}}),
            ),
            &context,
        )
        .await
        .unwrap();
    assert_eq!(pages.len(), 1);
    assert_eq!(
        pages[0].transport.receipt_id.as_ref().unwrap().as_str(),
        "rcpt_0"
    );
    assert_eq!(pages[0].parts.len(), 1);

    let failing = Arc::new(ScriptPaid {
        hits: AtomicUsize::new(0),
        fail_budget: true,
    });
    let (context, _puts) = ctx(
        Arc::new(ScriptEgress {
            hits: AtomicUsize::new(0),
            responses: Mutex::new(Vec::new()),
            seen: Mutex::new(Vec::new()),
        }),
        failing,
    );
    let error = accessor
        .access(
            request(
                Locator::ApiEndpoint {
                    endpoint: "/v1/search".into(),
                },
                json!({"service": "search"}),
            ),
            &context,
        )
        .await
        .unwrap_err();
    assert_eq!(error.class, ErrorClass::Budget);
    assert_eq!(error.message, "预算不足");
}

#[tokio::test]
async fn search_encodes_the_query() {
    let egress = Arc::new(ScriptEgress {
        hits: AtomicUsize::new(0),
        responses: Mutex::new(vec![HttpResponse {
            status: 200,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: Bytes::from_static(br#"{"results":[{"title":"hidden"}]}"#),
            final_url: "https://search.example/q".into(),
        }]),
        seen: Mutex::new(Vec::new()),
    });
    let (context, _puts) = ctx(
        egress.clone(),
        Arc::new(ScriptPaid {
            hits: AtomicUsize::new(0),
            fail_budget: false,
        }),
    );
    let pages = by_id("search")
        .unwrap()
        .access(
            request(
                Locator::Query {
                    query: "a b".into(),
                },
                json!({"endpoint": "https://search.example/?q={query}"}),
            ),
            &context,
        )
        .await
        .unwrap();
    assert_eq!(
        egress.seen.lock().await[0].url,
        "https://search.example/?q=a%20b"
    );
    assert_eq!(pages[0].parts.len(), 1);
    assert_eq!(
        pages[0].parts[0].bytes.size(),
        br#"{"results":[{"title":"hidden"}]}"#.len() as u64
    );
}

#[tokio::test]
async fn push_checks_token_rate_and_idempotency_without_listening() {
    let accessor = PushAccessor::new();
    let (context, puts) = ctx(
        Arc::new(ScriptEgress {
            hits: AtomicUsize::new(0),
            responses: Mutex::new(Vec::new()),
            seen: Mutex::new(Vec::new()),
        }),
        Arc::new(ScriptPaid {
            hits: AtomicUsize::new(0),
            fail_budget: false,
        }),
    );
    let spec = json!({"token": "secret", "ratePerSec": 1, "burst": 1});
    let policy = json!({});
    let now = Timestamp::parse("2026-10-20T00:00:00Z").unwrap();
    let denied = accessor
        .ingest(
            &spec,
            &Incoming {
                channel: "hooks".into(),
                headers: vec![("X-Push-Token".into(), "nope".into())],
                body: Bytes::from_static(b"{}"),
            },
            &context,
            &policy,
            now,
        )
        .await
        .unwrap_err();
    assert_eq!(denied.message, "unauthorized");
    let first = accessor
        .ingest(
            &spec,
            &Incoming {
                channel: "hooks".into(),
                headers: vec![
                    ("Authorization".into(), "Bearer secret".into()),
                    ("Idempotency-Key".into(), "k1".into()),
                    ("Content-Type".into(), "application/json".into()),
                ],
                body: Bytes::from_static(b"{\"a\":1}"),
            },
            &context,
            &policy,
            now,
        )
        .await
        .unwrap();
    let puts_before = puts.load(Ordering::SeqCst);
    let second = accessor
        .ingest(
            &spec,
            &Incoming {
                channel: "hooks".into(),
                headers: vec![
                    ("Authorization".into(), "Bearer secret".into()),
                    ("Idempotency-Key".into(), "k1".into()),
                ],
                body: Bytes::from_static(b"{\"a\":1}"),
            },
            &context,
            &policy,
            now,
        )
        .await
        .unwrap();
    assert_eq!(first[0].envelope_id, second[0].envelope_id);
    assert_eq!(puts.load(Ordering::SeqCst), puts_before);
    let limited = accessor
        .ingest(
            &spec,
            &Incoming {
                channel: "hooks".into(),
                headers: vec![("Authorization".into(), "Bearer secret".into())],
                body: Bytes::from_static(b"other"),
            },
            &context,
            &policy,
            now,
        )
        .await
        .unwrap_err();
    assert_eq!(limited.message, "rate-limit");
    let error = by_id("push")
        .unwrap()
        .access(
            request(
                Locator::PushChannel {
                    channel: "hooks".into(),
                },
                spec,
            ),
            &context,
        )
        .await
        .unwrap_err();
    assert_eq!(error.message, "inbound");
}

#[tokio::test]
async fn file_reads_inside_root_only() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("note.txt");
    std::fs::write(&file, "plain").unwrap();
    let (context, _puts) = ctx(
        Arc::new(ScriptEgress {
            hits: AtomicUsize::new(0),
            responses: Mutex::new(Vec::new()),
            seen: Mutex::new(Vec::new()),
        }),
        Arc::new(ScriptPaid {
            hits: AtomicUsize::new(0),
            fail_budget: false,
        }),
    );
    let root = dir.path().display().to_string();
    let pages = by_id("file")
        .unwrap()
        .access(
            request(
                Locator::Url {
                    url: "file://ignored".into(),
                },
                json!({"root": root, "path": "note.txt", "mediaType": "text/plain"}),
            ),
            &context,
        )
        .await
        .unwrap();
    assert_eq!(pages[0].parts[0].bytes.size(), 5);
    assert_eq!(pages[0].parts[0].media_type, "text/plain");
    let error = by_id("file")
        .unwrap()
        .access(
            request(
                Locator::Url {
                    url: "file://ignored".into(),
                },
                json!({"root": dir.path().display().to_string(), "path": ".."}),
            ),
            &context,
        )
        .await
        .unwrap_err();
    assert_eq!(error.message, "not-found");
}

#[tokio::test]
async fn websub_verifies_and_posts_a_subscription() {
    assert_eq!(
        verify(
            "hub.mode=subscribe&hub.topic=https%3A%2F%2Fexample.com%2Ffeed&hub.challenge=abc",
            "https://example.com/feed"
        )
        .status,
        200
    );
    assert_eq!(
        verify(
            "hub.mode=subscribe&hub.topic=https%3A%2F%2Fexample.com%2Ffeed&hub.challenge=abc",
            "https://example.com/feed"
        )
        .body,
        "abc"
    );
    assert_eq!(
        verify(
            "hub.mode=subscribe&hub.topic=other&hub.challenge=abc",
            "https://example.com/feed"
        )
        .status,
        404
    );
    let egress = Arc::new(ScriptEgress {
        hits: AtomicUsize::new(0),
        responses: Mutex::new(vec![HttpResponse {
            status: 202,
            headers: Vec::new(),
            body: Bytes::new(),
            final_url: "https://hub.example/".into(),
        }]),
        seen: Mutex::new(Vec::new()),
    });
    let (context, _puts) = ctx(
        egress.clone(),
        Arc::new(ScriptPaid {
            hits: AtomicUsize::new(0),
            fail_budget: false,
        }),
    );
    subscribe(
        &context,
        Instant::now() + Duration::from_secs(2),
        "https://hub.example/",
        "https://example.com/feed",
        "https://collector.example/callback",
    )
    .await
    .unwrap();
    let body = String::from_utf8(egress.seen.lock().await[0].body.to_vec()).unwrap();
    assert!(body.contains("hub.mode=subscribe"));
    assert!(body.contains("hub.callback=https%3A%2F%2Fcollector.example%2Fcallback"));
    let envelope = notify(
        &context,
        Notice {
            topic: "https://example.com/feed",
            headers: vec![("Content-Type".into(), "application/atom+xml".into())],
            body: Bytes::from_static(b"<feed/>"),
            policy: &json!({}),
        },
    )
    .unwrap();
    assert_eq!(envelope.parts.len(), 1);
    assert_eq!(envelope.parts[0].bytes.size(), 7);
}

#[tokio::test]
async fn browser_checks_robots_then_calls_the_proxy_engine() {
    let egress = Arc::new(ScriptEgress {
        hits: AtomicUsize::new(0),
        responses: Mutex::new(Vec::new()),
        seen: Mutex::new(Vec::new()),
    });
    let (context, _puts) = ctx(
        egress.clone(),
        Arc::new(ScriptPaid {
            hits: AtomicUsize::new(0),
            fail_budget: false,
        }),
    );
    let engine = ScriptEngine {
        hits: AtomicUsize::new(0),
        output: EngineOutput {
            final_url: "https://example.com/final".into(),
            status: 200,
            headers: vec![("Content-Type".into(), "text/html".into())],
            html: Some(b"<p>rendered</p>".to_vec()),
            network: Some(br#"[{"url":"https://example.com/a"}]"#.to_vec()),
            matched: vec![MatchedBody {
                url: "https://example.com/api".into(),
                media_type: "application/json".into(),
                body: br#"{"ok":true}"#.to_vec(),
            }],
            screenshot: Some(vec![1, 2, 3]),
            version: Some("fixture".into()),
            ..EngineOutput::default()
        },
    };
    let accessor = BrowserAccessor::new(engine, "http://127.0.0.1:9");
    let mut req = request(
        Locator::Url {
            url: "https://example.com/page".into(),
        },
        json!({}),
    );
    req.policy = Arc::new(
        json!({"capture": {"network": "matched-body", "screenshot": "on-render", "renditions": ["markdown"]}}),
    );
    let pages = accessor.access(req, &context).await.unwrap();
    assert_eq!(egress.hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        pages[0].transport.engine.as_ref().unwrap().version,
        "fixture"
    );
    assert_eq!(
        pages[0].transport.render_level,
        Some(strata_model::RenderLevel::L3)
    );
    assert!(
        pages[0]
            .parts
            .iter()
            .any(|part| part.role == PartRole::Document && part.media_type == "text/html")
    );
    assert!(
        pages[0]
            .parts
            .iter()
            .any(|part| part.role == PartRole::NetworkTrace)
    );
    assert!(
        pages[0]
            .parts
            .iter()
            .any(|part| part.role == PartRole::NetworkResponse)
    );
    assert!(
        pages[0]
            .parts
            .iter()
            .any(|part| part.role == PartRole::Screenshot)
    );
    assert!(
        pages[0]
            .parts
            .iter()
            .any(|part| part.role == PartRole::Metadata)
    );

    let blocked = request(
        Locator::Url {
            url: "https://example.com/page".into(),
        },
        json!({"mode": "cdp"}),
    );
    let error = accessor.validate(&blocked.spec).unwrap_err();
    assert_eq!(error.message, "未实测");
    let mut policy = request(
        Locator::Url {
            url: "https://example.com/page".into(),
        },
        json!({}),
    );
    policy.policy = Arc::new(json!({"capture": {"network": "all-bodies"}}));
    let before = egress.hits.load(Ordering::SeqCst);
    let error = accessor.access(policy, &context).await.unwrap_err();
    assert_eq!(error.message, "未实测");
    assert_eq!(egress.hits.load(Ordering::SeqCst), before);
}

#[tokio::test]
async fn moli_command_passes_the_proxy_to_a_stand_in() {
    let dir = tempfile::tempdir().unwrap();
    let args_path = dir.path().join("args");
    let program = dir.path().join("moli");
    let mut script = String::from("#!/bin/sh\nprintf '%s\\n' \"$@\" > '");
    script.push_str(&args_path.display().to_string());
    script.push_str("'\ncat <<'END'\n");
    script.push_str(r#"{"final_url":"https://example.com/final","status":200,"headers":[["Content-Type","text/html"]],"html":"<p>x</p>","redirect_chain":[],"version":"fixture"}"#);
    script.push_str("\nEND\n");
    std::fs::write(&program, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&program).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&program, permissions).unwrap();
    }
    let args = moli_args(&EngineRequest {
        url: "https://example.com/page".into(),
        proxy: "http://127.0.0.1:9".into(),
        headers: vec![("Accept".into(), "text/html".into())],
        wait_until: None,
        eval: None,
        js: true,
        frames: false,
        layout: false,
        resources: vec!["image".into()],
        trace: true,
        matched_body: false,
        block_private_networks: true,
    });
    assert!(
        args.windows(2)
            .any(|pair| pair[0] == "--http-proxy" && pair[1] == "http://127.0.0.1:9")
    );
    assert!(args.iter().any(|arg| arg == "--block-private-networks"));
    assert!(args.iter().any(|arg| arg == "--image"));

    let egress = Arc::new(ScriptEgress {
        hits: AtomicUsize::new(0),
        responses: Mutex::new(Vec::new()),
        seen: Mutex::new(Vec::new()),
    });
    let (context, _puts) = ctx(
        egress,
        Arc::new(ScriptPaid {
            hits: AtomicUsize::new(0),
            fail_budget: false,
        }),
    );
    let accessor = BrowserAccessor::new(MoliCommand { program }, "http://127.0.0.1:9");
    let pages = accessor
        .access(
            request(
                Locator::Url {
                    url: "https://example.com/page".into(),
                },
                json!({}),
            ),
            &context,
        )
        .await
        .unwrap();
    let recorded = std::fs::read_to_string(args_path).unwrap();
    assert!(recorded.contains("--http-proxy"));
    assert!(recorded.contains("http://127.0.0.1:9"));
    assert!(
        pages[0]
            .parts
            .iter()
            .any(|part| part.role == PartRole::Document)
    );
}

#[test]
fn registry_skips_browser() {
    assert!(by_id("browser").is_none());
    assert_eq!(by_id("file").unwrap().id(), "file");
}

struct Keep(std::sync::Mutex<Vec<Vec<u8>>>);

impl Staging for Keep {
    fn put(&self, bytes: &[u8]) -> Result<strata_model::PartRef, accessors::AccessError> {
        self.0.lock().expect("暂存").push(bytes.to_vec());
        Ok(strata_model::PartRef::from_bytes(bytes))
    }
}

#[tokio::test]
async fn browser_redacts_set_cookie_in_metadata_and_trace() {
    let kept = Arc::new(Keep(std::sync::Mutex::new(Vec::new())));
    let context = AccessContext {
        egress: Arc::new(ScriptEgress {
            hits: AtomicUsize::new(0),
            responses: Mutex::new(Vec::new()),
            seen: Mutex::new(Vec::new()),
        }),
        paid: Arc::new(ScriptPaid {
            hits: AtomicUsize::new(0),
            fail_budget: false,
        }),
        sessions: Arc::new(ScriptSessions),
        staging: kept.clone(),
        cancel: Arc::new(AtomicBool::new(false)),
    };
    let network = serde_json::json!({
        "events": [{
            "requestHeaders": [["Set-Cookie", "secret"], ["Accept", "text/html"]],
            "headers": [{"name": "Set-Cookie", "value": "secret"}]
        }]
    });
    let engine = ScriptEngine {
        hits: AtomicUsize::new(0),
        output: EngineOutput {
            final_url: "https://example.com/final".into(),
            status: 200,
            headers: vec![
                ("Content-Type".into(), "text/html".into()),
                ("Set-Cookie".into(), "secret".into()),
            ],
            html: Some(b"<p>rendered</p>".to_vec()),
            network: Some(serde_json::to_vec(&network).unwrap()),
            version: Some("fixture".into()),
            ..EngineOutput::default()
        },
    };
    let accessor = BrowserAccessor::new(engine, "http://127.0.0.1:9");
    let mut req = request(
        Locator::Url {
            url: "https://example.com/page".into(),
        },
        json!({}),
    );
    req.policy = Arc::new(json!({
        "raw": {"redact_headers": ["Set-Cookie"]},
        "capture": {"network": "trace"}
    }));
    let pages = accessor.access(req, &context).await.unwrap();
    assert_eq!(
        pages[0]
            .transport
            .headers
            .iter()
            .find(|(name, _)| name == "Set-Cookie")
            .unwrap()
            .1,
        "***"
    );
    let bodies = kept.0.lock().expect("暂存").clone();
    let stored = bodies
        .iter()
        .map(|body| String::from_utf8_lossy(body).into_owned())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!stored.contains("secret"));
    assert!(stored.contains("***"));
    assert!(
        pages[0]
            .parts
            .iter()
            .any(|part| part.role == PartRole::Metadata)
    );
    assert!(
        pages[0]
            .parts
            .iter()
            .any(|part| part.role == PartRole::NetworkTrace)
    );
}

#[tokio::test]
async fn concurrent_identical_push_keys_store_one_envelope() {
    let accessor = Arc::new(PushAccessor::new());
    let puts = Arc::new(AtomicUsize::new(0));
    let staging: Arc<dyn Staging> = Arc::new(MemStaging {
        puts: Arc::clone(&puts),
    });
    let egress: Arc<dyn EgressGateway> = Arc::new(ScriptEgress {
        hits: AtomicUsize::new(0),
        responses: Mutex::new(Vec::new()),
        seen: Mutex::new(Vec::new()),
    });
    let paid: Arc<dyn PaidGateway> = Arc::new(ScriptPaid {
        hits: AtomicUsize::new(0),
        fail_budget: false,
    });
    let sessions: Arc<dyn SessionPool> = Arc::new(ScriptSessions);
    let cancel = Arc::new(AtomicBool::new(false));
    let spec = json!({"token": "secret", "ratePerSec": 100, "burst": 20});
    let policy = json!({});
    let now = Timestamp::parse("2026-10-20T00:00:00Z").unwrap();
    let mut handles = Vec::new();
    for _ in 0..8 {
        let accessor = Arc::clone(&accessor);
        let staging = Arc::clone(&staging);
        let egress = Arc::clone(&egress);
        let paid = Arc::clone(&paid);
        let sessions = Arc::clone(&sessions);
        let cancel = Arc::clone(&cancel);
        let spec = spec.clone();
        let policy = policy.clone();
        handles.push(tokio::spawn(async move {
            let context = AccessContext {
                egress,
                paid,
                sessions,
                staging,
                cancel,
            };
            accessor
                .ingest(
                    &spec,
                    &Incoming {
                        channel: "hooks".into(),
                        headers: vec![
                            ("Authorization".into(), "Bearer secret".into()),
                            ("Idempotency-Key".into(), "same".into()),
                        ],
                        body: Bytes::from_static(b"{\"a\":1}"),
                    },
                    &context,
                    &policy,
                    now,
                )
                .await
                .unwrap()
        }));
    }
    let mut ids = Vec::new();
    for handle in handles {
        let envelopes = handle.await.unwrap();
        ids.push(envelopes[0].envelope_id.clone());
    }
    assert!(ids.iter().all(|id| id == &ids[0]));
    assert_eq!(puts.load(Ordering::SeqCst), 1);
}
