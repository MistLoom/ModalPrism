use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use bytes::Bytes;
use paid::{BudgetLimits, CallPolicy, Gateway, MemoryPersist, PaidTransport, Persist, UnknownMode};
use strata_model::{ErrorClass, Timestamp};
use tokio::sync::Notify;

use accessors::{AccessError, PaidCall, PaidResponse};

struct Hits {
    hits: Arc<AtomicUsize>,
    fail: bool,
    persist: Option<Arc<MemoryPersist>>,
}

#[async_trait]
impl PaidTransport for Hits {
    async fn send(&self, call: PaidCall) -> Result<PaidResponse, AccessError> {
        self.hits.fetch_add(1, Ordering::SeqCst);
        if let Some(persist) = &self.persist {
            let raw = persist.load("state").await.unwrap().unwrap();
            let text = String::from_utf8(raw).unwrap();
            assert!(text.contains("pending"), "{text}");
            assert!(text.contains(&call.service));
        }
        if self.fail {
            return Err(AccessError::new(ErrorClass::Transient, "timeout"));
        }
        Ok(PaidResponse {
            status: 200,
            headers: Vec::new(),
            body: Bytes::from_static(b"saved"),
            receipt_id: None,
        })
    }
}

fn call(params: serde_json::Value) -> PaidCall {
    PaidCall {
        service: "search".into(),
        endpoint: "/v1".into(),
        params,
        body: Bytes::new(),
        headers: Vec::new(),
    }
}

fn policy(limits: BudgetLimits, unknown: UnknownMode) -> CallPolicy {
    CallPolicy {
        unknown,
        window: std::time::Duration::from_secs(24 * 60 * 60),
        limits,
    }
}

#[tokio::test]
async fn budget_stops_the_second_send_and_replay_skips_transport() {
    let hits = Arc::new(AtomicUsize::new(0));
    let transport = Hits {
        hits: hits.clone(),
        fail: false,
        persist: None,
    };
    let gate = Gateway::new(
        MemoryPersist::default(),
        transport,
        policy(
            BudgetLimits {
                per_minute: Some(1),
                per_hour: Some(1),
                per_day: None,
            },
            UnknownMode::Manual,
        ),
    );
    let first = gate
        .call_at(call(serde_json::json!({"q": "a"})), 1_000)
        .await
        .unwrap();
    assert_eq!(first.body.as_ref(), b"saved");
    let id = first.receipt_id.as_ref().unwrap();
    let (service, key) = gate.stored_receipt(id.as_str()).await.unwrap().unwrap();
    assert_eq!(service, "search");
    assert_eq!(
        key,
        paid::logical_key("search", "/v1", &serde_json::json!({"q": "a"}), 0)
    );
    assert_ne!(key, id.as_str());
    let replay = gate
        .call_at(call(serde_json::json!({"q": "a"})), 1_200)
        .await
        .unwrap();
    assert_eq!(replay.body.as_ref(), b"saved");
    let second = gate
        .call_at(call(serde_json::json!({"q": "b"})), 1_500)
        .await
        .unwrap_err();
    assert_eq!(second.class, ErrorClass::Budget);
    assert_eq!(second.message, "预算不足");
    assert_eq!(second.retry_at, Timestamp::from_unix_millis(3_600_000).ok());
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn pending_is_durable_before_the_call_and_unknown_waits() {
    let hits = Arc::new(AtomicUsize::new(0));
    let persist = Arc::new(MemoryPersist::default());
    let transport = Hits {
        hits: hits.clone(),
        fail: true,
        persist: Some(persist.clone()),
    };
    let gate = Gateway::new(
        Shared(persist),
        transport,
        policy(BudgetLimits::default(), UnknownMode::RetryOnce),
    );
    let error = gate
        .call_at(call(serde_json::json!({"q": "a"})), 0)
        .await
        .unwrap_err();
    assert_eq!(error.class, ErrorClass::Transient);
    let waiting = gate
        .call_at(call(serde_json::json!({"q": "a"})), 1_000)
        .await
        .unwrap_err();
    assert_eq!(waiting.message, "unknown-wait");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let _ = gate
        .call_at(call(serde_json::json!({"q": "a"})), 3_600_001)
        .await
        .unwrap_err();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    let manual = gate
        .call_at(call(serde_json::json!({"q": "a"})), 3_600_002)
        .await
        .unwrap_err();
    assert_eq!(manual.class, ErrorClass::Permanent);
    assert_eq!(manual.message, "unknown-receipt");
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

struct Shared(Arc<MemoryPersist>);

#[async_trait]
impl Persist for Shared {
    async fn load(&self, key: &str) -> Result<Option<Vec<u8>>, AccessError> {
        self.0.load(key).await
    }

    async fn store(&self, key: &str, bytes: &[u8]) -> Result<(), AccessError> {
        self.0.store(key, bytes).await
    }
}

#[tokio::test]
async fn in_flight_key_is_not_sent_twice() {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let hits = Arc::new(AtomicUsize::new(0));
    let transport = Hold {
        hits: hits.clone(),
        started: started.clone(),
        release: release.clone(),
    };
    let gate = Arc::new(Gateway::new(
        MemoryPersist::default(),
        transport,
        policy(BudgetLimits::default(), UnknownMode::Manual),
    ));
    let first = gate.clone();
    let running = tokio::spawn(async move {
        first
            .call_at(call(serde_json::json!({"q": "a"})), 10_000)
            .await
    });
    started.notified().await;
    let second = gate
        .call_at(call(serde_json::json!({"q": "a"})), 10_100)
        .await
        .unwrap_err();
    assert_eq!(second.message, "in-flight");
    release.notify_one();
    running.await.unwrap().unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

struct Hold {
    hits: Arc<AtomicUsize>,
    started: Arc<Notify>,
    release: Arc<Notify>,
}

#[async_trait]
impl PaidTransport for Hold {
    async fn send(&self, _call: PaidCall) -> Result<PaidResponse, AccessError> {
        self.hits.fetch_add(1, Ordering::SeqCst);
        self.started.notify_one();
        self.release.notified().await;
        Ok(PaidResponse {
            status: 200,
            headers: Vec::new(),
            body: Bytes::from_static(b"ok"),
            receipt_id: None,
        })
    }
}
