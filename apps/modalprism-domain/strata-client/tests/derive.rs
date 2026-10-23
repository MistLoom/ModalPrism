//! 派生执行方：触发、血缘、稳态与失败退回。

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;
use strata_client::{DerivationInput, DerivationProducer, DerivationRun, Options, Strata};
use strata_model::{
    ApplyOpts, Body, ChangeSet, FacetKind, FacetRef, FacetSelector, LineageDir, LineageInput, Op,
    Producer, QueueName, Rid, Target, VersionSel,
};

const QUEUE: QueueName = QueueName::Free;
const LEASE: Duration = Duration::from_secs(30);

fn options(version: &str) -> Options {
    Options {
        derivations: vec![strata_client::DerivationSpec {
            name: "upper".into(),
            from: FacetSelector {
                kind: FacetKind::Text,
                variant: Some("body".into()),
                fields: vec!["text".into()],
            },
            produces: FacetKind::Text,
            variant: Some("upper".into()),
            producer: Producer {
                module: "test-derive".into(),
                version: version.into(),
                config_generation: 1,
            },
            params: json!({"mode": "ascii"}),
            queue: QUEUE,
            accessor: None,
            priority: 0,
        }],
        ..Options::default()
    }
}

fn input(identity: &str, text: &str) -> ChangeSet {
    ChangeSet {
        producer: Producer {
            module: "client-test".into(),
            version: "1".into(),
            config_generation: 1,
        },
        idempotency_key: None,
        ack: None,
        ops: vec![
            Op::Ensure {
                identity: identity.into(),
                ns: None,
                labels: BTreeMap::new(),
                r#as: "a".into(),
            },
            Op::PutFacet {
                target: Target::Alias("a".into()),
                kind: FacetKind::Text,
                variant: Some("body".into()),
                body: Body::Text { text: text.into() },
                media_type: Some("text/plain".into()),
                lineage: LineageInput {
                    inputs: Vec::new(),
                    params_hash: "input".into(),
                },
            },
        ],
    }
}

async fn write(strata: &Strata, identity: &str, text: &str) -> Rid {
    strata
        .apply(input(identity, text), ApplyOpts::default())
        .await
        .unwrap();
    strata.flush().await.unwrap();
    strata.resolve(identity).await.unwrap().unwrap()
}

fn derived(rid: &Rid) -> FacetRef {
    FacetRef::new(
        rid.clone(),
        FacetKind::Text,
        Some("upper".into()),
        VersionSel::Latest,
    )
    .unwrap()
}

fn source(rid: &Rid, version: u64) -> FacetRef {
    FacetRef::new(
        rid.clone(),
        FacetKind::Text,
        Some("body".into()),
        VersionSel::Exact(version),
    )
    .unwrap()
}

struct Upper;

#[async_trait]
impl DerivationProducer for Upper {
    async fn produce(&self, input: DerivationInput<'_>) -> Result<Option<Body>, String> {
        let text = input.input.text.as_deref().ok_or("输入不是文本")?;
        Ok(Some(Body::Text {
            text: text.to_uppercase(),
        }))
    }
}

struct Broken;

#[async_trait]
impl DerivationProducer for Broken {
    async fn produce(&self, _input: DerivationInput<'_>) -> Result<Option<Body>, String> {
        Err("model offline".into())
    }
}

#[tokio::test]
async fn derivation_runs_once_and_records_lineage() {
    let dir = tempfile::tempdir().unwrap();
    let strata = Strata::open(dir.path(), options("1")).await.unwrap();
    let rid = write(&strata, "ext:test:1", "hello").await;

    let run = strata
        .run_derivations(&Upper, QUEUE, 10, LEASE)
        .await
        .unwrap();
    assert_eq!(run.leased, 1);
    assert_eq!(run.produced, 1, "{run:?}");
    assert_eq!((run.skipped, run.failed), (0, 0));

    let facet = strata.get(&derived(&rid), false).await.unwrap().unwrap();
    assert_eq!(facet.text.as_deref(), Some("HELLO"));
    assert_eq!(facet.version, 1);

    // 血缘指向输入的具体版本，而不是“最新”。
    let graph = strata
        .lineage(&derived(&rid), LineageDir::Up, 1)
        .await
        .unwrap();
    assert_eq!(graph.edges.len(), 1);
    assert_eq!(graph.edges[0].to, source(&rid, 1));
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.module == "test-derive" && node.version == "1")
    );

    // 稳态：派生 facet 已覆盖输入，下一次发布不再入队。
    strata.flush().await.unwrap();
    let idle = strata
        .run_derivations(&Upper, QUEUE, 10, LEASE)
        .await
        .unwrap();
    assert_eq!(idle, DerivationRun::default());
}

#[tokio::test]
async fn failed_derivation_backs_off_before_retry() {
    let dir = tempfile::tempdir().unwrap();
    let strata = Strata::open(dir.path(), options("1")).await.unwrap();
    write(&strata, "ext:test:1", "hello").await;

    let run = strata
        .run_derivations(&Broken, QUEUE, 10, LEASE)
        .await
        .unwrap();
    assert_eq!(run.leased, 1);
    assert_eq!(run.failed, 1);
    assert_eq!(run.produced, 0);

    // nack 把任务放回队列并推迟 retry_at，立刻再领取拿不到。
    let again = strata.lease_derivations(QUEUE, 10, LEASE).await.unwrap();
    assert!(again.is_empty());
}

#[tokio::test]
async fn producer_version_bump_recomputes_identical_output() {
    let dir = tempfile::tempdir().unwrap();
    let rid = {
        let strata = Strata::open(dir.path(), options("1")).await.unwrap();
        let rid = write(&strata, "ext:test:1", "hello").await;
        strata
            .run_derivations(&Upper, QUEUE, 10, LEASE)
            .await
            .unwrap();
        rid
    };

    // 换代后同一输入的重算由下一次发布触发；新资源提供这次发布。
    let strata = Strata::open(dir.path(), options("2")).await.unwrap();
    write(&strata, "ext:test:2", "world").await;
    let run = strata
        .run_derivations(&Upper, QUEUE, 10, LEASE)
        .await
        .unwrap();
    assert_eq!(run.leased, 2);
    assert_eq!(run.produced, 2);

    // 输出不变也要留下新版本，否则触发端会一直认为派生 facet 过期。
    let facet = strata.get(&derived(&rid), false).await.unwrap().unwrap();
    assert_eq!(facet.text.as_deref(), Some("HELLO"));
    assert_eq!(facet.version, 2);
    let graph = strata
        .lineage(&derived(&rid), LineageDir::Up, 1)
        .await
        .unwrap();
    assert_eq!(graph.nodes[0].version, "2");

    // 换代重算后回到稳态：下一次发布只为新输入排队。
    write(&strata, "ext:test:3", "again").await;
    let steady = strata
        .run_derivations(&Upper, QUEUE, 10, LEASE)
        .await
        .unwrap();
    assert_eq!(steady.leased, 1);
    assert_eq!(steady.produced, 1);
}
