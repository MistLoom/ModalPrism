use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use planner::CadenceInput;
use planner::{
    Capacities, Classified, Handler, MemoryPersist, MemoryQueue, Persist, Planner, Quality, Queue,
    Receipts, SourcePlan, TaskSuccess, backoff, next_interval,
};
use serde_json::json;
use strata_model::{
    AccessTask, ErrorClass, Locator, QueueName, SourceId, TaskId, TaskReason, Timestamp,
};

fn now() -> Timestamp {
    Timestamp::parse("2026-10-20T00:00:00Z").unwrap()
}

fn plan() -> SourcePlan {
    SourcePlan {
        source_id: SourceId::new("source/blog").unwrap(),
        plan_id: None,
        locator: Locator::Url {
            url: "https://example.com/blog".into(),
        },
        accessor: "accessor/http".into(),
        queue: QueueName::Free,
        spec: json!({}),
        generation: 1,
        config_hash: "hash-a".into(),
        migration_generation: false,
        timeout: Duration::from_secs(30),
        paid: false,
        budget_requests: None,
        budget_window: Duration::ZERO,
        adaptive: false,
        interval: Duration::from_secs(30 * 60),
        floor: Duration::from_secs(15 * 60),
        ceiling: Duration::from_secs(6 * 60 * 60),
        batch_key: None,
        enabled: true,
        expected_period: Some(Duration::from_secs(7 * 24 * 60 * 60)),
        yield_drop_ratio: 0.3,
        failure_threshold: 5,
        daily_limit: None,
        one_shot: false,
        per_run_items: None,
        total_items: None,
        on_budget_exhausted: "skip".into(),
        stagnation_cycles: None,
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
            follows: Vec::new(),
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
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].task_id, released[0].task_id);
}

#[tokio::test]
async fn plan_total_items_cap_stops_dispatch() {
    let planner = Planner::new(MemoryQueue::new(), MemoryPersist::default());
    let mut p = plan();
    p.plan_id = Some("plan/cap".into());
    p.total_items = Some(1);
    let first = planner
        .tick(std::slice::from_ref(&p), &caps(), now())
        .await
        .unwrap();
    assert_eq!(first.len(), 1);
    // 记一次产出即达计划累计上限；释放租约后仍停派，并按 skipPolicy 记 exhausted 原因。
    planner
        .note_yield(p.source_id.as_str(), now(), 1)
        .await
        .unwrap();
    planner.shutdown(now()).await.unwrap();
    let second = planner
        .tick(std::slice::from_ref(&p), &caps(), now())
        .await
        .unwrap();
    assert!(second.is_empty(), "计划累计产出达 totalItems 上限后应停派");
    let status = planner.source_status(p.source_id.as_str()).await.unwrap();
    assert_eq!(status.skip_reason.as_deref(), Some("quota-exhausted"));
}

#[tokio::test]
async fn one_shot_plan_dispatches_once_then_completes() {
    let planner = Planner::new(MemoryQueue::new(), MemoryPersist::default());
    let mut p = plan();
    p.one_shot = true;
    p.plan_id = Some("plan/one".into());
    let first = planner
        .tick(std::slice::from_ref(&p), &caps(), now())
        .await
        .unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].plan_id.as_deref(), Some("plan/one"));
    // 释放租约也不应让一次性计划重新触发：首轮派发即转 completed。
    planner.shutdown(now()).await.unwrap();
    let second = planner
        .tick(std::slice::from_ref(&p), &caps(), now())
        .await
        .unwrap();
    assert!(second.is_empty(), "oneShot 计划派发一次后应转 completed");
}

#[tokio::test]
async fn nack_is_leased_again_when_retry_is_due() {
    let planner = Planner::new(MemoryQueue::new(), MemoryPersist::default());
    let first = planner
        .tick(std::slice::from_ref(&plan()), &caps(), now())
        .await
        .unwrap();
    let later = now().checked_add(Duration::from_secs(3600)).unwrap();
    planner
        .queue()
        .nack(&first[0].task_id, later, "transport")
        .await
        .unwrap();
    let mid = now().checked_add(Duration::from_secs(45)).unwrap();
    let waiting = planner
        .tick(std::slice::from_ref(&plan()), &caps(), mid)
        .await
        .unwrap();
    assert!(waiting.is_empty());
    let mut due = planner
        .tick(std::slice::from_ref(&plan()), &caps(), later)
        .await
        .unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].task_id, first[0].task_id);
    assert_eq!(due[0].attempt, 1);
    let handler = Script {
        hits: AtomicUsize::new(0),
        error: None,
        seen_cursor: std::sync::Mutex::new(None),
    };
    planner
        .run(&plan(), due.remove(0), &handler, &(), later, later)
        .await
        .unwrap();
    assert_eq!(handler.hits.load(Ordering::SeqCst), 1);
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

/// R11.3：迁移代即便 config_hash 翻转，也保留既有回填游标，不清空进度。
#[tokio::test]
async fn migration_generation_preserves_the_validator() {
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
    next.migration_generation = true;
    planner
        .run(&next, task, &handler, &(), now(), now())
        .await
        .unwrap();
    let seen = handler.seen_cursor.lock().unwrap().clone().unwrap();
    assert_eq!(
        seen,
        Some(json!({"page": 1, "configHash": "old"})),
        "迁移代应把旧游标原样交给处理器，而非清空"
    );
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

#[tokio::test]
async fn follow_task_is_leased_after_the_source_is_scheduled() {
    let planner = Planner::new(MemoryQueue::new(), MemoryPersist::default());
    let scheduled = planner
        .tick(std::slice::from_ref(&plan()), &caps(), now())
        .await
        .unwrap();
    assert_eq!(scheduled.len(), 1);
    let before = planner.source_status("source/blog").await.unwrap();
    let follow = AccessTask {
        task_id: TaskId::new("t_follow_1").unwrap(),
        source_id: SourceId::new("source/blog").unwrap(),
        plan_id: None,
        accessor: "accessor/http".into(),
        locator: Locator::Url {
            url: "https://example.com/blog/post".into(),
        },
        spec: json!({}),
        reason: TaskReason::Follow,
        priority: 20,
        queue: QueueName::Free,
        lease_until: now(),
        attempt: 0,
        batch_key: None,
        generation: 1,
    };
    planner
        .queue()
        .enqueue(std::slice::from_ref(&follow))
        .await
        .unwrap();
    let mut leased = planner
        .tick(std::slice::from_ref(&plan()), &caps(), now())
        .await
        .unwrap();
    assert_eq!(leased.len(), 1);
    assert_eq!(leased[0].task_id, follow.task_id);
    assert_eq!(leased[0].reason, TaskReason::Follow);
    let handler = Script {
        hits: AtomicUsize::new(0),
        error: None,
        seen_cursor: std::sync::Mutex::new(None),
    };
    planner
        .run(&plan(), leased.remove(0), &handler, &(), now(), now())
        .await
        .unwrap();
    let after = planner.source_status("source/blog").await.unwrap();
    assert_eq!(after.next_at, before.next_at);
    let again = planner
        .tick(std::slice::from_ref(&plan()), &caps(), now())
        .await
        .unwrap();
    assert!(again.is_empty());
}

fn paid_plan() -> SourcePlan {
    let mut plan = plan();
    plan.queue = QueueName::Paid;
    plan.paid = true;
    plan.budget_requests = Some(2);
    plan.budget_window = Duration::from_secs(60 * 60);
    plan
}

async fn finish(planner: &Planner<MemoryQueue, MemoryPersist>, task: AccessTask, at: Timestamp) {
    let handler = Script {
        hits: AtomicUsize::new(0),
        error: None,
        seen_cursor: std::sync::Mutex::new(None),
    };
    planner
        .run(&paid_plan(), task, &handler, &(), at, at)
        .await
        .unwrap();
}

struct Fixed {
    quality: Quality,
}

#[async_trait]
impl Handler for Fixed {
    async fn execute(
        &self,
        _task: &strata_model::AccessTask,
        _cursor: Option<serde_json::Value>,
    ) -> Result<TaskSuccess, Classified> {
        Ok(TaskSuccess {
            ops: Vec::new(),
            cursor: None,
            receipt: None,
            quality: self.quality,
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
            follows: Vec::new(),
        })
    }
}

/// v3 R18：web 类连续空命中达 `stagnation_cycles` 才升级为告警；未到阈值仅 suspect 不告警。
#[tokio::test]
async fn empty_hits_escalate_to_alert_after_stagnation_cycles() {
    let planner = Planner::new(MemoryQueue::new(), MemoryPersist::default());
    let mut p = plan();
    p.stagnation_cycles = Some(3);
    let handler = Fixed {
        quality: Quality::Empty,
    };
    for cycle in 1..=3 {
        planner.make_due(p.source_id.as_str(), now()).await.unwrap();
        let mut tasks = planner
            .tick(std::slice::from_ref(&p), &caps(), now())
            .await
            .unwrap();
        assert_eq!(tasks.len(), 1, "第 {cycle} 轮应派发一个任务");
        let task = tasks.remove(0);
        planner
            .run(&p, task, &handler, &(), now(), now())
            .await
            .unwrap();
        let status = planner.source_status(p.source_id.as_str()).await.unwrap();
        assert_eq!(status.health_status, "suspect", "空命中应判 suspect");
        if cycle < 3 {
            assert!(!status.alert, "未到阈值不应告警（第 {cycle} 轮）");
        }
    }
    let status = planner.source_status(p.source_id.as_str()).await.unwrap();
    assert!(status.alert, "连续 3 轮空命中达阈值应升级为告警");
}

/// v3 R18：非停滞类型（`stagnation_cycles=None`）单轮空命中判 suspect 但不告警。
#[tokio::test]
async fn empty_hits_without_stagnation_policy_do_not_alert() {
    let planner = Planner::new(MemoryQueue::new(), MemoryPersist::default());
    let mut p = plan();
    p.stagnation_cycles = None;
    let handler = Fixed {
        quality: Quality::Empty,
    };
    let mut tasks = planner
        .tick(std::slice::from_ref(&p), &caps(), now())
        .await
        .unwrap();
    let task = tasks.remove(0);
    planner
        .run(&p, task, &handler, &(), now(), now())
        .await
        .unwrap();
    let status = planner.source_status(p.source_id.as_str()).await.unwrap();
    assert_eq!(status.health_status, "suspect");
    assert!(!status.alert, "无停滞阈值的类型不因空命中告警");
}

#[tokio::test]
async fn paid_budget_counts_skips_and_resets_with_the_window() {
    let planner = Planner::new(MemoryQueue::new(), MemoryPersist::default());
    let plan = paid_plan();
    let start = now();
    let mut first = planner
        .tick(std::slice::from_ref(&plan), &caps(), start)
        .await
        .unwrap();
    assert_eq!(first.len(), 1);
    finish(&planner, first.remove(0), start).await;
    assert_eq!(
        planner
            .source_status("source/blog")
            .await
            .unwrap()
            .budget_used,
        1
    );
    planner.make_due("source/blog", start).await.unwrap();
    let mut second = planner
        .tick(std::slice::from_ref(&plan), &caps(), start)
        .await
        .unwrap();
    assert_eq!(second.len(), 1);
    finish(&planner, second.remove(0), start).await;
    assert_eq!(
        planner
            .source_status("source/blog")
            .await
            .unwrap()
            .budget_used,
        2
    );
    planner.make_due("source/blog", start).await.unwrap();
    let blocked = planner
        .tick(std::slice::from_ref(&plan), &caps(), start)
        .await
        .unwrap();
    assert!(blocked.is_empty());
    let exhausted = planner.source_status("source/blog").await.unwrap();
    assert_eq!(exhausted.skip_reason.as_deref(), Some("budget"));
    assert_eq!(exhausted.budget_used, 2);
    let mid = start.checked_add(Duration::from_secs(30 * 60)).unwrap();
    let still = planner
        .tick(std::slice::from_ref(&plan), &caps(), mid)
        .await
        .unwrap();
    assert!(still.is_empty());
    assert_eq!(
        planner
            .source_status("source/blog")
            .await
            .unwrap()
            .skip_reason
            .as_deref(),
        Some("budget")
    );
    let rolled = start.checked_add(Duration::from_secs(60 * 60)).unwrap();
    let resumed = planner
        .tick(std::slice::from_ref(&plan), &caps(), rolled)
        .await
        .unwrap();
    assert_eq!(resumed.len(), 1);
    let restored = planner.source_status("source/blog").await.unwrap();
    assert!(restored.skip_reason.is_none());
    assert_eq!(restored.budget_used, 1);
}
