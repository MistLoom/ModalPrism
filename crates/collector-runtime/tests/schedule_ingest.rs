use strata_model::{AccessTask, ErrorClass, Locator, QueueName, SourceId, TaskId, TaskReason, Timestamp};

fn schedule_task() -> AccessTask {
    AccessTask {
        task_id: TaskId::new("t_schedule").unwrap(),
        source_id: SourceId::new("source/demo").unwrap(),
        accessor: "accessor/http".into(),
        locator: Locator::Url {
            url: "https://example.test/feed".into(),
        },
        spec: serde_json::json!({}),
        reason: TaskReason::Schedule,
        priority: 10,
        queue: QueueName::Free,
        lease_until: Timestamp::now(),
        attempt: 0,
        batch_key: None,
        generation: 1,
    }
}

#[test]
fn schedule_empty_envelopes_is_permanent_failure() {
    let err = collector_runtime::execute::schedule_ingest_error(
        &schedule_task(),
        &[],
        &[],
        planner::Quality::Empty,
        false,
    )
    .expect_err("应失败");
    assert_eq!(err.class, ErrorClass::Permanent);
    assert_eq!(err.message, "empty-fetch");
}

#[test]
fn schedule_blocked_is_permanent_failure() {
    let err = collector_runtime::execute::schedule_ingest_error(
        &schedule_task(),
        &[],
        &[],
        planner::Quality::Blocked,
        false,
    )
    .expect_err("应失败");
    assert_eq!(err.class, ErrorClass::Permanent);
    assert_eq!(err.message, "blocked");
}
