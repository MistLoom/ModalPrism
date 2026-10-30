//! 队列生命周期：取消、重试、删除、清理、统计与游标分页。
//!
//! 全部走 `Gateway` 门面，只验证状态机与查询行为，不涉及执行侧。

use std::collections::BTreeSet;
use std::time::Duration;

use strata_gateway::task::{STATUS_CANCELLED, STATUS_DONE, STATUS_QUEUED, TaskFilter};
use strata_gateway::{BlobBackend, Error, Gateway, Options};
use strata_model::{AccessTask, Locator, QueueName, SourceId, TaskId, TaskReason, Timestamp};

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

fn task(id: &str, locator: &str) -> AccessTask {
    AccessTask {
        task_id: TaskId::new(id).unwrap(),
        source_id: SourceId::new("source_it").unwrap(),
        plan_id: None,
        accessor: "accessor_it".into(),
        locator: Locator::Url {
            url: locator.into(),
        },
        spec: serde_json::json!({"token": "secret-never-projected"}),
        reason: TaskReason::Manual,
        priority: 30,
        queue: QueueName::Free,
        lease_until: Timestamp::now(),
        attempt: 0,
        batch_key: None,
        generation: 1,
    }
}

fn id(raw: &str) -> TaskId {
    TaskId::new(raw).unwrap()
}

#[tokio::test]
async fn cancel_moves_active_tasks_to_cancelled_and_ack_stays_idempotent() {
    let (_dir, gateway) = open().await;
    let queued = task("t_queued", "https://it.example/queued");
    let leased = task("t_leased", "https://it.example/leased");
    gateway.enqueue(&[queued, leased]).await.unwrap();
    let now = Timestamp::now();
    let taken = gateway
        .lease(QueueName::Free, 1, Duration::from_secs(60), now)
        .await
        .unwrap();
    assert_eq!(taken.len(), 1);

    // 排队中与持有租约的都能取消。
    assert!(gateway.cancel_task(&id("t_queued")).await.unwrap());
    assert!(gateway.cancel_task(&id("t_leased")).await.unwrap());
    // 已经终态的不再变动。
    assert!(!gateway.cancel_task(&id("t_queued")).await.unwrap());

    let cancelled = gateway.task_row(&id("t_queued")).await.unwrap().unwrap();
    assert_eq!(cancelled.status, STATUS_CANCELLED);
    assert!(cancelled.lease_until.is_none());
    assert_eq!(cancelled.last_error.as_deref(), Some("已取消"));

    // 取消后 worker 已取到的数据照常提交，但状态不回到完成。
    gateway.ack(&id("t_queued")).await.unwrap();
    gateway.ack(&id("t_queued")).await.unwrap();
    let after = gateway.task_row(&id("t_queued")).await.unwrap().unwrap();
    assert_eq!(after.status, STATUS_CANCELLED);

    let stats = gateway.task_stats().await.unwrap();
    assert_eq!(stats.by_status.get(STATUS_CANCELLED), Some(&2));
    assert_eq!(stats.total, 2);
}

#[tokio::test]
async fn cancelled_task_frees_the_locator_slot() {
    let (_dir, gateway) = open().await;
    let first = task("t_first", "https://it.example/same");
    assert_eq!(
        gateway
            .enqueue(std::slice::from_ref(&first))
            .await
            .unwrap()
            .len(),
        1
    );
    // 同一目标还有活跃任务，重复入队会被去重吞掉。
    let duplicate = task("t_dup", "https://it.example/same");
    assert!(gateway.enqueue(&[duplicate]).await.unwrap().is_empty());

    gateway.cancel_task(&first.task_id).await.unwrap();

    let retry = task("t_retry", "https://it.example/same");
    let inserted = gateway.enqueue(&[retry]).await.unwrap();
    assert_eq!(inserted.len(), 1, "取消后同一目标应当可以重新入队");
    assert!(
        gateway
            .task_active("source_it", &serde_json::to_string(&first.locator).unwrap())
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn active_id_resolves_the_live_task_when_enqueue_is_deduped() {
    let (_dir, gateway) = open().await;
    let first = task("t_live", "https://it.example/live");
    gateway.enqueue(std::slice::from_ref(&first)).await.unwrap();

    // 同一定位键重复入队会被去重吞掉：新任务不入库，回读应为 None。
    let dup = task("t_dup2", "https://it.example/live");
    assert!(
        gateway
            .enqueue(std::slice::from_ref(&dup))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(gateway.task_row(&id("t_dup2")).await.unwrap().is_none());

    // active_id 返回真正活跃的那条，供采集路径解析出可 run 的任务，避免「任务不存在」。
    let key = serde_json::to_string(&first.locator).unwrap();
    assert_eq!(
        gateway
            .active_task_id("source_it", &key)
            .await
            .unwrap()
            .map(|task_id| task_id.to_string()),
        Some("t_live".to_string())
    );

    // 取消后活跃位释放，active_id 不再返回旧任务。
    gateway.cancel_task(&id("t_live")).await.unwrap();
    assert!(
        gateway
            .active_task_id("source_it", &key)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn requeue_restores_cancelled_and_clears_lease() {
    let (_dir, gateway) = open().await;
    gateway
        .enqueue(&[task("t_again", "https://it.example/again")])
        .await
        .unwrap();
    let now = Timestamp::now();
    gateway
        .lease(QueueName::Free, 1, Duration::from_secs(60), now)
        .await
        .unwrap();
    gateway.cancel_task(&id("t_again")).await.unwrap();

    assert!(gateway.requeue_task(&id("t_again")).await.unwrap());
    let row = gateway.task_row(&id("t_again")).await.unwrap().unwrap();
    assert_eq!(row.status, STATUS_QUEUED);
    assert!(row.lease_until.is_none());
    assert!(row.retry_at.is_none());

    // 恢复后立刻可以再租出去。
    let taken = gateway
        .lease(
            QueueName::Free,
            1,
            Duration::from_secs(60),
            Timestamp::now(),
        )
        .await
        .unwrap();
    assert_eq!(taken.len(), 1);
    assert_eq!(taken[0].task_id, id("t_again"));
}

#[tokio::test]
async fn remove_only_accepts_terminal_tasks() {
    let (_dir, gateway) = open().await;
    gateway
        .enqueue(&[task("t_live", "https://it.example/live")])
        .await
        .unwrap();
    let error = gateway.remove_task(&id("t_live")).await.unwrap_err();
    assert!(matches!(error, Error::TaskState(_)), "{error}");

    gateway.cancel_task(&id("t_live")).await.unwrap();
    assert!(gateway.remove_task(&id("t_live")).await.unwrap());
    assert!(gateway.task_row(&id("t_live")).await.unwrap().is_none());
    assert!(matches!(
        gateway.remove_task(&id("t_live")).await.unwrap_err(),
        Error::UnknownTask(_)
    ));
}

#[tokio::test]
async fn purge_only_touches_terminal_tasks() {
    let (_dir, gateway) = open().await;
    gateway
        .enqueue(&[
            task("t_keep", "https://it.example/keep"),
            task("t_done", "https://it.example/done"),
            task("t_cancel", "https://it.example/cancel"),
        ])
        .await
        .unwrap();
    gateway.ack(&id("t_done")).await.unwrap();
    gateway.cancel_task(&id("t_cancel")).await.unwrap();

    // 指定非终态状态直接拒绝，避免误删在途任务。
    let error = gateway
        .purge_tasks(&TaskFilter {
            status: Some(STATUS_QUEUED.into()),
            ..TaskFilter::default()
        })
        .await
        .unwrap_err();
    assert!(matches!(error, Error::InvalidFilter(_)), "{error}");

    let removed = gateway.purge_tasks(&TaskFilter::default()).await.unwrap();
    assert_eq!(removed, 2);
    let left = gateway
        .list_tasks(&TaskFilter::default(), None, 10)
        .await
        .unwrap();
    assert_eq!(left.rows.len(), 1);
    assert_eq!(left.rows[0].task_id, "t_keep");
    assert_eq!(left.rows[0].status, STATUS_QUEUED);
}

#[tokio::test]
async fn list_cursor_walks_every_row_once() {
    let (_dir, gateway) = open().await;
    let mut tasks = Vec::new();
    for index in 0..5 {
        tasks.push(task(
            &format!("t_page_{index}"),
            &format!("https://it.example/page-{index}"),
        ));
    }
    gateway.enqueue(&tasks).await.unwrap();

    let all = gateway
        .list_tasks(&TaskFilter::default(), None, 50)
        .await
        .unwrap();
    assert_eq!(all.total, 5);
    assert_eq!(all.rows.len(), 5);
    assert!(all.next_cursor.is_none());
    // 新任务在前：按创建时间倒序。
    let expected: Vec<String> = all.rows.iter().map(|row| row.task_id.clone()).collect();

    let mut seen: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = gateway
            .list_tasks(&TaskFilter::default(), cursor.as_deref(), 2)
            .await
            .unwrap();
        assert_eq!(page.total, 5, "同一条件的总数不随翻页变化");
        assert!(page.rows.len() <= 2);
        seen.extend(page.rows.iter().map(|row| row.task_id.clone()));
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(seen, expected);
    assert_eq!(
        seen.iter().collect::<BTreeSet<_>>().len(),
        5,
        "翻页不应重复"
    );

    // 统计与列表口径一致。
    let stats = gateway.task_stats().await.unwrap();
    assert_eq!(stats.total, 5);
    assert_eq!(stats.by_status.get(STATUS_QUEUED), Some(&5));
    assert_eq!(stats.by_reason.get("manual"), Some(&5));
    assert_eq!(stats.by_queue.get("free"), Some(&5));
    assert!(stats.oldest_queued_at.is_some());
}

#[tokio::test]
async fn done_tasks_count_as_terminal_but_never_lease_again() {
    let (_dir, gateway) = open().await;
    gateway
        .enqueue(&[task("t_finish", "https://it.example/finish")])
        .await
        .unwrap();
    gateway.ack(&id("t_finish")).await.unwrap();
    let row = gateway.task_row(&id("t_finish")).await.unwrap().unwrap();
    assert_eq!(row.status, STATUS_DONE);

    let leased = gateway
        .lease(
            QueueName::Free,
            1,
            Duration::from_secs(60),
            Timestamp::now(),
        )
        .await
        .unwrap();
    assert!(leased.is_empty(), "已完成的任务不应再被租出去");
    assert!(
        gateway
            .lease_id(&id("t_finish"), Duration::from_secs(60), Timestamp::now())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn keyword_filter_matches_id_source_and_error() {
    let (dir, gateway) = open().await;
    gateway
        .enqueue(&[task("t_alpha", "https://it.example/alpha")])
        .await
        .unwrap();
    let needle = gateway
        .list_tasks(
            &TaskFilter {
                query: Some("t_alph".into()),
                ..TaskFilter::default()
            },
            None,
            10,
        )
        .await
        .unwrap();
    assert_eq!(needle.rows.len(), 1);
    assert_eq!(needle.total, 1);

    let none = gateway
        .list_tasks(
            &TaskFilter {
                query: Some("没有这个关键字".into()),
                ..TaskFilter::default()
            },
            None,
            10,
        )
        .await
        .unwrap();
    assert!(none.rows.is_empty());
    drop(dir);
}
