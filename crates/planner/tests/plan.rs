use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use planner::CadenceInput;
use planner::{
    Capacities, Classified, Handler, MemoryPersist, MemoryQueue, Persist, Planner, Quality, Queue,
    Receipts, SourcePlan, TaskSuccess, backoff, next_interval,
};
use serde_json::json;
use strata_model::{ErrorClass, Locator, QueueName, SourceId, TaskReason, Timestamp};

fn now() -> Timestamp {
    Timestamp::parse("2026-10-20T00:00:00Z").unwrap()
}

fn plan() -> SourcePlan {
    SourcePlan {
        source_id: SourceId::new("source/blog").unwrap(),
        locator: Locator::Url {
            url: "https://example.com/blog".into(),
        },
        accessor: "accessor/http".into(),
        queue: QueueName::Free,
        spec: json!({}),
        generation: 1,
        config_hash: "hash-a".into(),
        timeout: Duration::from_secs(30),
        paid: false,
        adaptive: false,
        interval: Duration::from_secs(30 * 60),
        floor: Duration::from_secs(15 * 60),
        ceiling: Duration::from_secs(6 * 60 * 60),
        batch_key: None,
        enabled: true,
        expected_period: Some(Duration::from_secs(7 * 24 * 60 * 60)),
        yield_drop_ratio: 0.3,
        failure_threshold: 5,
    }
}

fn caps() -> Capacities {
    Capacities {
        free: 2,
        paid: 1,
        render: 1,
        push: 1,
    }
}

#[test]
fn cadence_uses_seven_day_yield() {
    let interval = next_interval(&CadenceInput {
        yield_7d: 21,
        floor: Duration::from_secs(15 * 60),
        ceiling: Duration::from_secs(6 * 60 * 60),
        signal: Duration::ZERO,
        paid: false,
        adaptive: true,
        fixed: Duration::from_secs(30 * 60),
    });
    assert_eq!(interval, Duration::from_secs(160 * 60));
    let paid = next_interval(&CadenceInput {
        yield_7d: 210,
        floor: Duration::from_secs(15 * 60),
        ceiling: Duration::from_secs(6 * 60 * 60),
        signal: Duration::ZERO,
        paid: true,
        adaptive: true,
        fixed: Duration::from_secs(30 * 60),
    });
    assert_eq!(paid, Duration::from_secs(60 * 60));
    let empty = next_interval(&CadenceInput {
        yield_7d: 0,
        floor: Duration::from_secs(15 * 60),
        ceiling: Duration::from_secs(2 * 60 * 60),
        signal: Duration::ZERO,
        paid: false,
        adaptive: true,
        fixed: Duration::from_secs(30 * 60),
    });
    assert_eq!(empty, Duration::from_secs(2 * 60 * 60));
}

#[test]
fn backoff_caps_and_honors_retry_after() {
    let interval = Duration::from_secs(30 * 60);
    assert_eq!(backoff(interval, 1, None), Duration::from_secs(60 * 60));
    assert_eq!(
        backoff(interval, 10, None),
        Duration::from_secs(6 * 60 * 60)
    );
    assert_eq!(
        backoff(interval, 1, Some(Duration::from_secs(8 * 60 * 60))),
        Duration::from_secs(8 * 60 * 60)
    );
}

struct Script {
    hits: AtomicUsize,
    error: Option<Classified>,
    seen_cursor: std::sync::Mutex<Option<Option<serde_json::Value>>>,
}

#[async_trait]
impl Handler for Script {
    async fn execute(
        &self,
        _task: &strata_model::AccessTask,
        cursor: Option<serde_json::Value>,
    ) -> Result<TaskSuccess, Classified> {
        self.hits.fetch_add(1, Ordering::SeqCst);
        *self.seen_cursor.lock().unwrap() = Some(cursor);
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        Ok(TaskSuccess {
            ops: Vec::new(),
            cursor: Some(json!({"page": 2})),
            receipt: Some(planner::ReceiptBody {
                id: "rcpt_1".into(),
                service: "search".into(),
                logical_key: "abc".into(),
            }),
            quality: Quality::Ok,
            selector_miss: false,
            blocked: false,
            ttl: None,
            retry_after: None,
            cache_max_age: None,
            expires: None,
            new_resources: 0,
            render_level: None,
            cost: 0,
            adapters: Vec::new(),
        })
    }
}

#[tokio::test]
async fn lease_blocks_a_second_dispatch_until_shutdown() {
    let queue = MemoryQueue::new();
    let planner = Planner::new(queue, MemoryPersist::default());
    let first = planner
        .tick(std::slice::from_ref(&plan()), &caps(), now())
        .await
        .unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].reason, TaskReason::Schedule);
    let second = planner
        .tick(std::slice::from_ref(&plan()), &caps(), now())
        .await
        .unwrap();
    assert!(second.is_empty());
    let released = planner.shutdown(now()).await.unwrap();
    assert_eq!(released.len(), 1);
    let again = planner
        .tick(std::slice::from_ref(&plan()), &caps(), now())
        .await
        .unwrap();
    assert!(again.is_empty());
    planner
        .queue()
        .lease_id(&released[0].task_id, Duration::from_secs(45), now())
        .await
        .unwrap();
}

#[tokio::test]
async fn budget_does_not_mark_the_source_failed() {
    let planner = Planner::new(MemoryQueue::new(), MemoryPersist::default());
    let task = planner
        .tick(std::slice::from_ref(&plan()), &caps(), now())
        .await
        .unwrap()
        .remove(0);
    let handler = Script {
        hits: AtomicUsize::new(0),
        error: Some(Classified {
            class: ErrorClass::Budget,
            message: "预算不足".into(),
            retry_after: None,
            retry_at: Some(Timestamp::parse("2026-10-20T00:01:00Z").unwrap()),
        }),
        seen_cursor: std::sync::Mutex::new(None),
    };
    planner
        .run(&plan(), task, &handler, &(), now(), now())
        .await
        .unwrap();
    let ops = planner
        .snapshot(now(), Duration::from_secs(1))
        .await
        .unwrap();
    assert!(
        ops.iter()
            .all(|op| !matches!(op, strata_model::Op::SetState { scope, .. } if scope == "health"))
    );
}

#[tokio::test]
async fn transient_degrades_and_permanent_fails() {
    let planner = Planner::new(MemoryQueue::new(), MemoryPersist::default());
    let task = planner
        .tick(std::slice::from_ref(&plan()), &caps(), now())
        .await
        .unwrap()
        .remove(0);
    let handler = Script {
        hits: AtomicUsize::new(0),
        error: Some(Classified {
            class: ErrorClass::Transient,
            message: "timeout".into(),
            retry_after: None,
            retry_at: None,
        }),
        seen_cursor: std::sync::Mutex::new(None),
    };
    planner
        .run(&plan(), task, &handler, &(), now(), now())
        .await
        .unwrap();
    let ops = planner
        .snapshot(now(), Duration::from_secs(1))
        .await
        .unwrap();
    let health = ops
        .iter()
        .find_map(|op| match op {
            strata_model::Op::SetState { scope, value, .. } if scope == "health" => {
                Some(value.clone())
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(health["status"], "degraded");
    assert_eq!(handler.hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn accepted_cursor_is_ahead_of_visible_and_receipt_shares_the_changeset() {
    let queue = MemoryQueue::new();
    let planner = Planner::new(queue, MemoryPersist::default());
    let task = planner
        .tick(std::slice::from_ref(&plan()), &caps(), now())
        .await
        .unwrap()
        .remove(0);
    let handler = Script {
        hits: AtomicUsize::new(0),
        error: None,
        seen_cursor: std::sync::Mutex::new(None),
    };
    let seen = Seen(std::sync::Mutex::new(None));
    planner
        .run(&plan(), task, &handler, &seen, now(), now())
        .await
        .unwrap();
    assert_eq!(seen.0.lock().unwrap().as_deref(), Some("rcpt_1"));
    let accepted = planner
        .queue()
        .accepted_cursor("source/blog")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(accepted["page"], 2);
    assert_eq!(accepted["configHash"], "hash-a");
    assert!(
        planner
            .queue()
            .visible_cursor("source/blog")
            .await
            .unwrap()
            .is_none()
    );
    planner.queue().publish().await;
    assert_eq!(
        planner
            .queue()
            .visible_cursor("source/blog")
            .await
            .unwrap()
            .unwrap()["page"],
        2
    );
}

#[tokio::test]
async fn changed_config_hash_drops_the_validator() {
    let queue = MemoryQueue::new();
    let planner = Planner::new(queue, MemoryPersist::default());
    let task = planner
        .tick(std::slice::from_ref(&plan()), &caps(), now())
        .await
        .unwrap()
        .remove(0);
    let seeded = strata_model::ChangeSet {
        producer: strata_model::Producer {
            module: "planner".into(),
            version: "1".into(),
            config_generation: 1,
        },
        idempotency_key: None,
        ack: None,
        ops: vec![strata_model::Op::SetState {
            scope: "cursor".into(),
            key: "source/blog".into(),
            value: json!({"page": 1, "configHash": "old"}),
            expect_version: None,
        }],
    };
    planner.queue().apply(seeded).await.unwrap();
    let handler = Script {
        hits: AtomicUsize::new(0),
        error: None,
        seen_cursor: std::sync::Mutex::new(None),
    };
    let mut next = plan();
    next.config_hash = "hash-b".into();
    next.generation = 2;
    planner
        .run(&next, task, &handler, &(), now(), now())
        .await
        .unwrap();
    assert_eq!(handler.seen_cursor.lock().unwrap().clone().unwrap(), None);
}

struct Seen(std::sync::Mutex<Option<String>>);

#[async_trait]
impl Receipts for Seen {
    async fn complete(&self, receipt_id: &str) -> Result<(), planner::PlannerError> {
        *self.0.lock().unwrap() = Some(receipt_id.to_string());
        Ok(())
    }
}

#[tokio::test]
async fn earlier_next_at_is_dispatched_first() {
    let persist = MemoryPersist::default();
    let state = json!({
        "sources": {
            "source/late": {
                "next_at": "2026-10-19T23:30:00Z",
                "interval_ms": 1_800_000,
                "generation": 1,
                "last_yield_at": null,
                "last_recompute": "2026-10-20"
            },
            "source/early": {
                "next_at": "2026-10-19T23:00:00Z",
                "interval_ms": 1_800_000,
                "generation": 1,
                "last_yield_at": null,
                "last_recompute": "2026-10-20"
            }
        },
        "health": {},
        "yields": {},
        "last_snapshot": null
    });
    persist
        .store("state", &serde_json::to_vec(&state).unwrap())
        .await
        .unwrap();
    let planner = Planner::new(MemoryQueue::new(), persist);
    let mut caps = caps();
    caps.free = 1;
    let mut late = plan();
    late.source_id = SourceId::new("source/late").unwrap();
    late.locator = Locator::Url {
        url: "https://example.com/late".into(),
    };
    let mut early = plan();
    early.source_id = SourceId::new("source/early").unwrap();
    early.locator = Locator::Url {
        url: "https://example.com/early".into(),
    };
    let tasks = planner.tick(&[late, early], &caps, now()).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].source_id.as_str(), "source/early");
}
