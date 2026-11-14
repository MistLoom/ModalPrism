//! 向量写入阶段验收：未开启不写、派生路径才放行、同哈希不重复堆、墓碑清理。
//!
//! 全部走 `Gateway` 门面 + 直接读 `g_vectors` 计数，验证提交语义而不是执行侧。

use std::collections::BTreeMap;

use strata_gateway::data;
use strata_gateway::db::Db;
use strata_gateway::{Backend, BlobBackend, Error, Gateway, Options};
use strata_model::{
    ApplyOpts, Body, ChangeSet, FacetKind, LineageInput, Op, Producer, QueueName, SourceId,
    Target, TaskId, TaskReason, WaitFor, Locator,
};

fn changeset(ack: Option<&str>, ops: Vec<Op>) -> ChangeSet {
    ChangeSet {
        producer: Producer {
            module: "derive-embedding".into(),
            version: "1".into(),
            config_generation: 1,
        },
        idempotency_key: None,
        ack: ack.map(|id| TaskId::new(id).unwrap()),
        ops,
    }
}

/// 带来源标签的 Ensure：来源标签会落成 `resources.source_id`，是向量触发范围过滤的依据。
fn ensure_for(alias: &str, source: &str) -> Op {
    let mut labels = BTreeMap::new();
    labels.insert("source".to_string(), source.to_string());
    Op::Ensure {
        identity: format!("ext:vec-it:{alias}"),
        ns: None,
        labels,
        r#as: alias.into(),
    }
}

fn ensure(alias: &str) -> Op {
    Op::Ensure {
        identity: format!("ext:vec-it:{alias}"),
        ns: None,
        labels: BTreeMap::new(),
        r#as: alias.into(),
    }
}

fn text_facet(alias: &str) -> Op {
    Op::PutFacet {
        target: Target::Alias(alias.into()),
        kind: FacetKind::Text,
        variant: None,
        body: Body::Text {
            text: "需要被向量化的正文".into(),
        },
        media_type: Some("text/plain".into()),
        lineage: LineageInput {
            inputs: Vec::new(),
            params_hash: "params".into(),
        },
    }
}

/// 嵌入片段：由"派生任务"写入（changeset 带 ack），正文必须带模型与向量。
fn embedding_facet(alias: &str, model: &str, vector: &[f32]) -> Op {
    Op::PutFacet {
        target: Target::Alias(alias.into()),
        kind: FacetKind::Embedding,
        variant: Some("hash-64-v1".into()),
        body: Body::Struct {
            value: serde_json::json!({
                "model": model,
                "vector": vector,
            }),
        },
        media_type: None,
        lineage: LineageInput {
            inputs: Vec::new(),
            params_hash: "params".into(),
        },
    }
}

fn tombstone(alias: &str) -> Vec<Op> {
    vec![
        ensure(alias),
        Op::Tombstone {
            target: Target::Alias(alias.into()),
            reason: "内容被移除".into(),
        },
    ]
}

async fn open() -> (tempfile::TempDir, Gateway, Db) {
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
    let db = Db::open(&Backend::default(), dir.path()).await.unwrap();
    (dir, gateway, db)
}

/// 派生写入要有可确认的任务：先入队，changeset 带 ack。
async fn enqueue_task(gateway: &Gateway, id: &str) {
    gateway
        .enqueue(&[strata_model::AccessTask {
            task_id: TaskId::new(id).unwrap(),
            source_id: SourceId::new("vectorize").unwrap(),
            plan_id: None,
            accessor: "derive-embedding".into(),
            locator: Locator::Query {
                query: "strata://vectorize/it".into(),
            },
            spec: serde_json::json!({}),
            reason: TaskReason::Detail,
            priority: 10,
            queue: QueueName::Render,
            lease_until: strata_model::Timestamp::now(),
            attempt: 0,
            batch_key: None,
            pipeline: None,
            vectorize: false,
            generation: 1,
        }])
        .await
        .unwrap();
}

async fn flush(gateway: &Gateway, changeset: ChangeSet) {
    let accepted = gateway.apply(changeset, ApplyOpts::default()).await.unwrap();
    assert_eq!(accepted.status, WaitFor::Accepted);
    gateway.flush().await.unwrap();
}

#[tokio::test]
async fn direct_apply_embedding_is_rejected_not_silently_indexed() {
    let (_dir, gateway, db) = open().await;
    // 未指定嵌入的直接 apply 维持既有约束：明确拒绝，索引里没有向量。
    let error = gateway
        .apply(
            changeset(None, vec![ensure("a"), embedding_facet("a", "hash-64-v1", &[0.5, -0.5])]),
            ApplyOpts::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, Error::EmbeddingUnspecified), "{error}");
    assert_eq!(data::count(&db, "vectors").await.unwrap(), 0);
}

#[tokio::test]
async fn derivation_path_writes_vector_incrementally_without_duplicates() {
    let (_dir, gateway, db) = open().await;
    enqueue_task(&gateway, "t_vec_1").await;
    flush(
        &gateway,
        changeset(
            Some("t_vec_1"),
            vec![ensure("a"), text_facet("a"), embedding_facet("a", "hash-64-v1", &[0.5, -0.5])],
        ),
    )
    .await;
    assert_eq!(data::count(&db, "vectors").await.unwrap(), 1);
    assert_eq!(data::count(&db, "facets").await.unwrap(), 2);

    // 同内容哈希（同模型同向量）再次写入：更新即跳过，不重复堆条目。
    enqueue_task(&gateway, "t_vec_2").await;
    flush(
        &gateway,
        changeset(
            Some("t_vec_2"),
            vec![ensure("a"), embedding_facet("a", "hash-64-v1", &[0.5, -0.5])],
        ),
    )
    .await;
    assert_eq!(
        data::count(&db, "vectors").await.unwrap(),
        1,
        "同内容哈希不得新增向量行"
    );

    // 内容变化（不同向量）：追加新版本，检索取最新。
    enqueue_task(&gateway, "t_vec_3").await;
    flush(
        &gateway,
        changeset(
            Some("t_vec_3"),
            vec![ensure("a"), embedding_facet("a", "hash-64-v1", &[0.9, 0.1])],
        ),
    )
    .await;
    assert_eq!(data::count(&db, "vectors").await.unwrap(), 2);
}

#[tokio::test]
async fn tombstone_removes_vectors_with_the_content() {
    let (_dir, gateway, db) = open().await;
    enqueue_task(&gateway, "t_vec_1").await;
    flush(
        &gateway,
        changeset(
            Some("t_vec_1"),
            vec![ensure("a"), text_facet("a"), embedding_facet("a", "hash-64-v1", &[0.5, -0.5])],
        ),
    )
    .await;
    assert_eq!(data::count(&db, "vectors").await.unwrap(), 1);

    // 内容被删除/移除：对应向量必须去掉，寿命与已落盘内容一致。
    flush(&gateway, changeset(None, tombstone("a"))).await;
    assert_eq!(
        data::count(&db, "vectors").await.unwrap(),
        0,
        "墓碑之后向量必须被清理"
    );
}

#[tokio::test]
async fn malformed_embedding_bodies_fail_loudly() {
    let (_dir, gateway, db) = open().await;
    enqueue_task(&gateway, "t_vec_1").await;

    // 缺 model：明确报错，不得把空值冒充已嵌入。
    let missing_model = vec![ensure("a"), Op::PutFacet {
        target: Target::Alias("a".into()),
        kind: FacetKind::Embedding,
        variant: Some("hash-64-v1".into()),
        body: Body::Struct {
            value: serde_json::json!({"vector": [0.1, 0.2]}),
        },
        media_type: None,
        lineage: LineageInput {
            inputs: Vec::new(),
            params_hash: "params".into(),
        },
    }];
    let error = gateway
        .apply(changeset(Some("t_vec_1"), missing_model), ApplyOpts::default())
        .await
        .unwrap_err();
    assert!(matches!(error, Error::EmbeddingUnspecified), "{error}");

    // 空向量同样拒绝。
    enqueue_task(&gateway, "t_vec_2").await;
    let error = gateway
        .apply(
            changeset(Some("t_vec_2"), vec![ensure("a"), embedding_facet("a", "hash-64-v1", &[])]),
            ApplyOpts::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, Error::EmbeddingUnspecified), "{error}");
    assert_eq!(data::count(&db, "vectors").await.unwrap(), 0);
}

/// 范围隔离：向量触发必须以指定信源为对象，只入队该信源名下单元的任务；
/// 其它信源的内容不进本次范围，空信源不产生任何任务——不存在全局触发。
#[tokio::test]
async fn trigger_vectorize_is_scoped_to_the_requested_source() {
    let (_dir, gateway, _db) = open().await;
    enqueue_task(&gateway, "t_scope_seed").await;
    // 信源 A 与 B 各自的内容（文本规范单元）。
    flush(
        &gateway,
        changeset(
            Some("t_scope_seed"),
            vec![
                ensure_for("a", "source/alpha"),
                text_facet("a"),
                ensure_for("b", "source/beta"),
                text_facet("b"),
            ],
        ),
    )
    .await;

    // 以 A 为对象触发：任务只覆盖 A 名下的单元。
    let tasks = gateway.trigger_vectorize("source/alpha").await.unwrap();
    assert_eq!(tasks.len(), 1, "只应入队信源甲的 1 个文本单元: {tasks:?}");

    // 空/未指定信源：不入队任何任务。
    let none = gateway.trigger_vectorize("").await.unwrap();
    assert!(none.is_empty(), "空信源不得触发任何入队");

    // 换 B 为对象：覆盖的是 B 的单元，与 A 互不越界。
    let tasks_b = gateway.trigger_vectorize("source/beta").await.unwrap();
    assert_eq!(tasks_b.len(), 1, "只应入队信源乙的 1 个文本单元: {tasks_b:?}");
    assert_ne!(
        tasks[0].locator_key(), tasks_b[0].locator_key(),
        "两个信源的派生任务不得指向同一单元"
    );
}
