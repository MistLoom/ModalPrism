use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::Bytes;
use serde_json::json;
use strata_model::{
    EnvelopeId, ErrorClass, Locator, PartId, PartRef, PartRole, RawEnvelope, RawPart, Signals,
    SourceId, TaskId, Timestamp, Transport,
};

use accessors::{
    AccessContext, AccessHints, AccessRequest, Accessor, AccessorCapabilities, CaptureNetwork,
    EgressGateway, HttpRequest, HttpResponse, PaidCall, PaidGateway, PaidResponse, Session,
    SessionPool, Staging, redact_envelope,
};

struct MemEgress;

#[async_trait]
impl EgressGateway for MemEgress {
    async fn request(&self, request: HttpRequest) -> Result<HttpResponse, accessors::AccessError> {
        Ok(HttpResponse {
            status: 200,
            headers: vec![
                ("Content-Type".into(), "text/html".into()),
                ("Authorization".into(), "secret".into()),
            ],
            body: request.body,
            final_url: request.url,
        })
    }
}

struct MemPaid;

#[async_trait]
impl PaidGateway for MemPaid {
    async fn call(&self, _request: PaidCall) -> Result<PaidResponse, accessors::AccessError> {
        Err(accessors::AccessError::new(ErrorClass::Budget, "预算不足"))
    }
}

struct MemSessions;

#[async_trait]
impl SessionPool for MemSessions {
    async fn get(&self, _profile: &str) -> Result<Session, accessors::AccessError> {
        Ok(Session {
            headers: Vec::new(),
            cookie: None,
        })
    }
}

struct MemStaging;

impl Staging for MemStaging {
    fn put(&self, bytes: &[u8]) -> Result<PartRef, accessors::AccessError> {
        Ok(PartRef::from_bytes(bytes))
    }
}

struct StaticAccessor {
    capabilities: AccessorCapabilities,
}

#[async_trait]
impl Accessor for StaticAccessor {
    fn id(&self) -> &str {
        "test"
    }

    fn version(&self) -> &str {
        "1"
    }

    fn capabilities(&self) -> &AccessorCapabilities {
        &self.capabilities
    }

    fn validate(&self, spec: &serde_json::Value) -> Result<(), accessors::AccessError> {
        if spec.is_object() {
            Ok(())
        } else {
            Err(accessors::AccessError::new(
                ErrorClass::Config,
                "spec 必须是对象",
            ))
        }
    }

    async fn access(
        &self,
        request: AccessRequest,
        ctx: &AccessContext,
    ) -> Result<Vec<RawEnvelope>, accessors::AccessError> {
        if ctx.is_cancelled() {
            return Err(accessors::AccessError::cancelled());
        }
        let response = ctx
            .egress
            .request(HttpRequest {
                method: "GET".into(),
                url: "https://example.com/a".into(),
                headers: Vec::new(),
                body: Bytes::new(),
            })
            .await?;
        let part = ctx.staging.put(&response.body)?;
        let mut envelope = RawEnvelope {
            envelope_id: EnvelopeId::new("env-1").unwrap(),
            locator: request.locator,
            final_locator: Some(Locator::Url {
                url: response.final_url,
            }),
            transport: Transport {
                protocol: "http".into(),
                status: Some(response.status),
                headers: response.headers,
                started_at: Timestamp::parse("2026-10-20T00:00:00Z").unwrap(),
                duration: Duration::from_secs(1),
                bytes_in: part.size(),
                not_modified: false,
                render_level: None,
                engine: None,
                receipt_id: None,
            },
            parts: vec![RawPart {
                part_id: PartId::new("p1").unwrap(),
                role: PartRole::Document,
                media_type: "text/html".into(),
                charset: None,
                locator: None,
                bytes: part,
                meta: Default::default(),
            }],
            cursor_candidate: None,
            signals: Signals::default(),
        };
        redact_envelope(&mut envelope, &request.policy);
        Ok(vec![envelope])
    }
}

fn context(cancel: bool) -> AccessContext {
    AccessContext {
        egress: Arc::new(MemEgress),
        paid: Arc::new(MemPaid),
        sessions: Arc::new(MemSessions),
        staging: Arc::new(MemStaging),
        cancel: Arc::new(AtomicBool::new(cancel)),
    }
}

fn request() -> AccessRequest {
    AccessRequest {
        task_id: TaskId::new("task-1").unwrap(),
        source_id: SourceId::new("source/demo").unwrap(),
        locator: Locator::Url {
            url: "https://example.com/a".into(),
        },
        spec: json!({}),
        cursor: None,
        hints: AccessHints::default(),
        policy: Arc::new(json!({"raw": {"redact_headers": ["authorization", "cookie"]}})),
        deadline: Instant::now(),
    }
}

#[tokio::test]
async fn accessor_uses_egress_and_staging_and_redacts_headers() {
    let accessor = StaticAccessor {
        capabilities: AccessorCapabilities {
            protocols: vec!["http".into()],
            conditional: false,
            render: false,
            layout: false,
            capture_network: CaptureNetwork::None,
            interact: false,
            paid: None,
            batchable: false,
            inbound: false,
        },
    };
    let envelopes = accessor.access(request(), &context(false)).await.unwrap();
    assert_eq!(envelopes.len(), 1);
    assert_eq!(envelopes[0].parts[0].bytes.size(), 0);
    assert_eq!(
        envelopes[0].transport.headers,
        vec![
            ("Content-Type".into(), "text/html".into()),
            ("Authorization".into(), "***".into()),
        ]
    );
}

#[tokio::test]
async fn cancelled_access_is_transient() {
    let accessor = StaticAccessor {
        capabilities: AccessorCapabilities {
            protocols: vec!["http".into()],
            conditional: false,
            render: false,
            layout: false,
            capture_network: CaptureNetwork::None,
            interact: false,
            paid: None,
            batchable: false,
            inbound: false,
        },
    };
    let error = accessor
        .access(request(), &context(true))
        .await
        .unwrap_err();
    assert_eq!(error.class, ErrorClass::Transient);
    assert_eq!(error.message, "cancelled");
    assert!(error.source.is_none());
}

#[tokio::test]
async fn paid_port_returns_budget_without_rewriting_it() {
    let paid = MemPaid;
    let error = paid
        .call(PaidCall {
            service: "search".into(),
            endpoint: "/q".into(),
            params: json!({}),
            body: Bytes::new(),
            headers: Vec::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(error.class, ErrorClass::Budget);
    assert_eq!(error.to_string(), "预算不足");
}
