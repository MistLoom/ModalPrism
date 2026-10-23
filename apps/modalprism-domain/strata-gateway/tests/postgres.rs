//! PostgreSQL 后端集成验证。默认忽略，配置实例后手动运行：
//!
//! `STRATA_TEST_POSTGRES_URL=postgres://… cargo test -p strata-gateway --features postgres --test postgres -- --ignored`
//!
//! 固定使用 `strata_gateway_it` schema，每次运行前重建，结束后删除。连接串需要
//! 建 schema 的权限。pgvector 可选：未安装时迁移跳过扩展，文本路径照常验证。

#![cfg(feature = "postgres")]

use std::collections::BTreeMap;

use strata_gateway::{Backend, BlobBackend, Gateway, Options, Request};
use strata_model::{
    ApplyOpts, Body, ChangeSet, FacetKind, LineageInput, Op, Producer, QueryFrom, Search, Target,
    WaitFor,
};

const SCHEMA: &str = "strata_gateway_it";

fn url() -> String {
    std::env::var("STRATA_TEST_POSTGRES_URL")
        .expect("需要 STRATA_TEST_POSTGRES_URL 指向可建 schema 的 PostgreSQL")
}

/// 让连接默认落进测试 schema，并保留 `public` 供 pgvector 类型解析。
fn scoped(base: &str, schema: &str) -> String {
    let separator = if base.contains('?') { '&' } else { '?' };
    format!("{base}{separator}options=-csearch_path%3D{schema}%2Cpublic")
}

fn changeset() -> ChangeSet {
    ChangeSet {
        producer: Producer {
            module: "gateway-it".into(),
            version: "1".into(),
            config_generation: 1,
        },
        idempotency_key: None,
        ack: None,
        ops: vec![
            Op::Ensure {
                identity: "ext:pg:a".into(),
                ns: None,
                labels: BTreeMap::new(),
                r#as: "a".into(),
            },
            Op::PutFacet {
                target: Target::Alias("a".into()),
                kind: FacetKind::Text,
                variant: None,
                body: Body::Text {
                    text: "postgres full text".into(),
                },
                media_type: Some("text/plain".into()),
                lineage: LineageInput {
                    inputs: Vec::new(),
                    params_hash: "params".into(),
                },
            },
        ],
    }
}

#[tokio::test]
#[ignore = "需要 STRATA_TEST_POSTGRES_URL"]
async fn postgres_roundtrip_and_text_search() {
    let base = url();
    let admin = sqlx::PgPool::connect(&base).await.unwrap();
    sqlx::query(&format!("DROP SCHEMA IF EXISTS {SCHEMA} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();
    sqlx::query(&format!("CREATE SCHEMA {SCHEMA}"))
        .execute(&admin)
        .await
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let gateway = Gateway::open(
        dir.path(),
        Options {
            backend: Backend::Postgres {
                url: scoped(&base, SCHEMA),
            },
            blobs: BlobBackend::Memory,
            ..Options::default()
        },
    )
    .await
    .unwrap();

    let accepted = gateway
        .apply(changeset(), ApplyOpts::default())
        .await
        .unwrap();
    assert_eq!(accepted.status, WaitFor::Accepted);
    assert!(accepted.snapshot.is_none());

    let seq = gateway.flush().await.unwrap().unwrap();
    assert_eq!(seq, 1);
    let snapshot = gateway.snapshot(None).await.unwrap().unwrap();
    assert_eq!(snapshot.seq, 1);

    let mut text = Request::new(QueryFrom::Resources);
    text.search = Some(Search {
        text: Some("postgres".into()),
        vector: None,
        variant: None,
        k: 5,
    });
    let response = gateway.query(text).await.unwrap();
    assert_eq!(response.rows.len(), 1, "{:?}", response.rows);
    assert!(!response.rows[0].rid.is_empty());
    assert_eq!(response.snapshot, Some(1));

    // 测试 schema 里没有 g_vec_2；没有建表时向量检索必须报错，而不是返回空页。
    let mut vector = Request::new(QueryFrom::Resources);
    vector.search = Some(Search {
        text: None,
        vector: Some(vec![0.1, 0.2]),
        variant: None,
        k: 5,
    });
    let error = gateway.query(vector).await.unwrap_err();
    assert!(error.to_string().contains("g_vec_2"), "{error}");

    drop(gateway);
    sqlx::query(&format!("DROP SCHEMA {SCHEMA} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();
}
