//! 控制库上的租约、accepted 游标和同一变更集里的回执。

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use paid::{CallPolicy, Gateway, PaidTransport, Persist as PaidPersist};
use planner::{
    Capacities, Classified, Handler, Planner, PlannerError, Quality, Queue, Receipts, SourcePlan,
    TaskSuccess,
};
use serde_json::{Value, json};
use strata_client::{Options, Strata};
use strata_model::{
    AccessTask, ApplyOpts, Body, ChangeSet, ErrorClass, FacetKind, LineageInput, Locator, Op,
    QueueName, SourceId, Target, TaskId, Timestamp,
};

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
        budget_requests: None,
        budget_window: Duration::ZERO,
        adaptive: false,
        interval: Duration::from_secs(30 * 60),
        floor: Duration::from_secs(15 * 60),
        ceiling: Duration::from_secs(6 * 60 * 60),
        batch_key: None,
        enabled: true,
        expected_period: None,
        yield_drop_ratio: 0.3,
        failure_threshold: 5,
    }
}

fn plan_err(error: impl std::error::Error + Send + Sync + 'static) -> PlannerError {
    PlannerError::with_source("控制存储", error)
}

struct Hot {
    strata: Arc<Strata>,
    kind: &'static str,
}

#[async_trait]
impl planner::Persist for Hot {
    async fn load(&self, key: &str) -> Result<Option<Vec<u8>>, PlannerError> {
        match self.strata.hot_get(self.kind, key).await {
            Ok(Some(value)) => Ok(Some(value.to_string().into_bytes())),
            Ok(None) => Ok(None),
            Err(error) => Err(plan_err(error)),
        }
    }

    async fn store(&self, key: &str, bytes: &[u8]) -> Result<(), PlannerError> {
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|error| PlannerError::with_source("控制热数据读写失败", error))?;
        self.strata
            .hot_put(self.kind, key, &value)
            .await
            .map_err(plan_err)
    }
}

#[async_trait]
impl PaidPersist for Hot {
    async fn load(&self, key: &str) -> Result<Option<Vec<u8>>, accessors::AccessError> {
        match self.strata.hot_get(self.kind, key).await {
            Ok(Some(value)) => Ok(Some(value.to_string().into_bytes())),
            Ok(None) => Ok(None),
            Err(error) => Err(accessors::AccessError::with_source(
                ErrorClass::Transient,
                "控制热数据读写失败",
                error,
            )),
        }
    }

    async fn store(&self, key: &str, bytes: &[u8]) -> Result<(), accessors::AccessError> {
        let value: Value = serde_json::from_slice(bytes).map_err(|error| {
            accessors::AccessError::with_source(ErrorClass::Transient, "控制热数据读写失败", error)
        })?;
        self.strata
            .hot_put(self.kind, key, &value)
            .await
            .map_err(|error| {
                accessors::AccessError::with_source(
                    ErrorClass::Transient,
                    "控制热数据读写失败",
                    error,
                )
            })
    }
}

struct StrataQueue(Arc<Strata>);

#[async_trait]
impl Queue for StrataQueue {
    async fn enqueue(&self, tasks: &[AccessTask]) -> Result<(), PlannerError> {
        self.0.enqueue(tasks).await.map_err(plan_err)
    }

    async fn lease_id(
        &self,
        task_id: &TaskId,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<AccessTask, PlannerError> {
        self.0
            .lease_id(task_id, lease_for, now)
            .await
            .map_err(plan_err)
    }

    async fn nack(
        &self,
        task_id: &TaskId,
        retry_at: Timestamp,
        error: &str,
    ) -> Result<(), PlannerError> {
        self.0
            .nack(task_id, retry_at, error)
            .await
            .map_err(plan_err)
    }

    async fn release_leases(&self) -> Result<Vec<AccessTask>, PlannerError> {
        self.0.release_leases().await.map_err(plan_err)
    }

    async fn release_source_leases(&self, source: &str) -> Result<(), PlannerError> {
        self.0.release_source_leases(source).await.map_err(plan_err)
    }

    async fn lease_ready(
        &self,
        source: &str,
        locator_key: &str,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<Option<AccessTask>, PlannerError> {
        self.0
            .lease_ready(source, locator_key, lease_for, now)
            .await
            .map_err(plan_err)
    }

    async fn lease_follows(
        &self,
        queue: QueueName,
        count: usize,
        lease_for: Duration,
        now: Timestamp,
    ) -> Result<Vec<AccessTask>, PlannerError> {
        self.0
            .lease_follows(queue, count, lease_for, now)
            .await
            .map_err(plan_err)
    }

    async fn active(&self, source: &str, locator_key: &str) -> Result<bool, PlannerError> {
        self.0
            .task_active(source, locator_key)
            .await
            .map_err(plan_err)
    }

    async fn leased_count(&self, queue: QueueName, now: Timestamp) -> Result<usize, PlannerError> {
        self.0.leased_count(queue, now).await.map_err(plan_err)
    }

    async fn apply(&self, changeset: ChangeSet) -> Result<(), PlannerError> {
        self.0
            .apply(changeset, ApplyOpts::default())
            .await
            .map_err(plan_err)?;
        Ok(())
    }

    async fn accepted_cursor(&self, source: &str) -> Result<Option<Value>, PlannerError> {
        Ok(self
            .0
            .accepted_state("cursor", source)
            .await
            .map_err(plan_err)?
            .map(|state| state.value))
    }

    async fn visible_cursor(&self, source: &str) -> Result<Option<Value>, PlannerError> {
        Ok(self
            .0
            .state("cursor", source)
            .await
            .map_err(plan_err)?
            .map(|state| state.value))
    }
}

struct Echo;

#[async_trait]
impl PaidTransport for Echo {
    async fn send(
        &self,
        _call: accessors::PaidCall,
    ) -> Result<accessors::PaidResponse, accessors::AccessError> {
        Ok(accessors::PaidResponse {
            status: 200,
            headers: Vec::new(),
            body: Bytes::from_static(b"ok"),
            receipt_id: None,
        })
    }
}

struct Write {
    receipt_id: String,
    logical_key: String,
}

#[async_trait]
impl Handler for Write {
    async fn execute(
        &self,
        _task: &AccessTask,
        _cursor: Option<Value>,
    ) -> Result<TaskSuccess, Classified> {
        Ok(TaskSuccess {
            ops: vec![
                Op::Ensure {
                    identity: "ext:doi:10.1000/p5".into(),
                    ns: None,
                    labels: Default::default(),
                    r#as: "a".into(),
                },
                Op::PutFacet {
                    target: Target::Alias("a".into()),
                    kind: FacetKind::Text,
                    variant: None,
                    body: Body::Text {
                        text: "phase-5".into(),
                    },
                    media_type: Some("text/plain".into()),
                    lineage: LineageInput {
                        inputs: Vec::new(),
                        params_hash: "params".into(),
                    },
                },
            ],
            cursor: Some(json!({"page": 2})),
            receipt: Some(planner::ReceiptBody {
                id: self.receipt_id.clone(),
                service: "search".into(),
                logical_key: self.logical_key.clone(),
            }),
            quality: Quality::Ok,
            selector_miss: false,
            blocked: false,
            ttl: None,
            retry_after: None,
            cache_max_age: None,
            expires: None,
            new_resources: 1,
            render_level: None,
            cost: 0,
            adapters: Vec::new(),
            follows: Vec::new(),
        })
    }
}

struct Finish {
    gate: Gateway<Hot, Echo>,
    strata: Arc<Strata>,
}

#[async_trait]
impl Receipts for Finish {
    async fn complete(&self, receipt_id: &str) -> Result<(), PlannerError> {
        let accepted = self
            .strata
            .accepted_state("receipt", receipt_id)
            .await
            .map_err(plan_err)?
            .unwrap();
        assert_eq!(accepted.value["status"], "completed");
        assert!(
            self.strata
                .state("receipt", receipt_id)
                .await
                .map_err(plan_err)?
                .is_none()
        );
        assert!(
            self.strata
                .resolve("ext:doi:10.1000/p5")
                .await
                .map_err(plan_err)?
                .is_none()
        );
        self.gate
            .complete(receipt_id)
            .await
            .map_err(|error| PlannerError::with_source("回执", error))
    }
}

#[tokio::test]
async fn control_store_keeps_lease_cursor_and_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let strata = Arc::new(Strata::open(dir.path(), Options::default()).await.unwrap());
    let params = json!({"q": "a"});
    let logical_key = paid::logical_key("search", "/v1", &params, 0);
    let gate = Gateway::new(
        Hot {
            strata: strata.clone(),
            kind: "paid",
        },
        Echo,
        CallPolicy::default(),
    );
    let paid = gate
        .call_at(
            accessors::PaidCall {
                service: "search".into(),
                endpoint: "/v1".into(),
                params,
                body: Bytes::new(),
                headers: Vec::new(),
            },
            1_000,
        )
        .await
        .unwrap();
    let receipt_id = paid.receipt_id.unwrap().as_str().to_string();
    assert_eq!(
        gate.status(&receipt_id).await.unwrap().as_deref(),
        Some("ready")
    );

    let planner = Planner::new(
        StrataQueue(strata.clone()),
        Hot {
            strata: strata.clone(),
            kind: "planner",
        },
    );
    let caps = Capacities {
        free: 2,
        paid: 1,
        render: 1,
        push: 1,
    };
    let first = planner
        .tick(std::slice::from_ref(&plan()), &caps, now())
        .await
        .unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].attempt, 0);
    assert!(
        planner
            .tick(std::slice::from_ref(&plan()), &caps, now())
            .await
            .unwrap()
            .is_empty()
    );

    let released = planner.shutdown(now()).await.unwrap();
    assert_eq!(released.len(), 1);
    assert_eq!(released[0].attempt, 0);
    assert_eq!(
        strata.leased_count(QueueName::Free, now()).await.unwrap(),
        0
    );
    let task = planner
        .queue()
        .lease_id(&released[0].task_id, Duration::from_secs(45), now())
        .await
        .unwrap();

    let finish = Finish {
        gate,
        strata: strata.clone(),
    };
    planner
        .run(
            &plan(),
            task,
            &Write {
                receipt_id: receipt_id.clone(),
                logical_key: logical_key.clone(),
            },
            &finish,
            now(),
            now(),
        )
        .await
        .unwrap();
    assert_eq!(
        finish.gate.status(&receipt_id).await.unwrap().as_deref(),
        Some("completed")
    );
    let hot = strata.hot_get("paid", "state").await.unwrap().unwrap();
    assert!(hot.to_string().contains("completed"));
    assert!(
        !strata
            .task_active(
                "source/blog",
                &serde_json::to_string(&plan().locator).unwrap()
            )
            .await
            .unwrap()
    );

    let accepted = strata
        .accepted_state("cursor", "source/blog")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(accepted.value["page"], 2);
    assert_eq!(accepted.value["configHash"], "hash-a");
    assert!(
        strata
            .state("cursor", "source/blog")
            .await
            .unwrap()
            .is_none()
    );
    let receipt = strata
        .accepted_state("receipt", &receipt_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.value["logicalKey"], logical_key);
    assert!(
        strata
            .state("receipt", &receipt_id)
            .await
            .unwrap()
            .is_none()
    );

    strata.flush().await.unwrap();
    assert_eq!(
        strata
            .state("cursor", "source/blog")
            .await
            .unwrap()
            .unwrap()
            .value["page"],
        2
    );
    assert_eq!(
        strata
            .state("receipt", &receipt_id)
            .await
            .unwrap()
            .unwrap()
            .value["status"],
        "completed"
    );
    assert!(
        strata
            .resolve("ext:doi:10.1000/p5")
            .await
            .unwrap()
            .is_some()
    );
}

struct Boom;

#[async_trait]
impl Handler for Boom {
    async fn execute(
        &self,
        _task: &AccessTask,
        _cursor: Option<Value>,
    ) -> Result<TaskSuccess, Classified> {
        Err(Classified {
            class: ErrorClass::Transient,
            message: "transport".into(),
            retry_after: None,
            retry_at: None,
        })
    }
}

#[tokio::test]
async fn nack_on_the_control_store_runs_again_when_due() {
    let dir = tempfile::tempdir().unwrap();
    let strata = Arc::new(Strata::open(dir.path(), Options::default()).await.unwrap());
    let planner = Planner::new(
        StrataQueue(strata.clone()),
        Hot {
            strata: strata.clone(),
            kind: "planner",
        },
    );
    let caps = Capacities {
        free: 2,
        paid: 1,
        render: 1,
        push: 1,
    };
    let first = planner
        .tick(std::slice::from_ref(&plan()), &caps, now())
        .await
        .unwrap();
    planner
        .run(&plan(), first[0].clone(), &Boom, &(), now(), now())
        .await
        .unwrap();
    let early = now().checked_add(Duration::from_secs(45)).unwrap();
    assert!(
        planner
            .tick(std::slice::from_ref(&plan()), &caps, early)
            .await
            .unwrap()
            .is_empty()
    );
    let due_at = now().checked_add(Duration::from_secs(60 * 60)).unwrap();
    let due = planner
        .tick(std::slice::from_ref(&plan()), &caps, due_at)
        .await
        .unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].task_id, first[0].task_id);
    assert_eq!(due[0].attempt, 1);
}
