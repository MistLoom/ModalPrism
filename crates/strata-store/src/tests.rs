use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use strata_model::{
    ApplyOpts, Body, ChangeSet, FacetKind, LineageInput, Op, Producer, Target, WaitFor,
};

use crate::{Engine, Options};

fn producer() -> Producer {
    Producer {
        module: "store-test".into(),
        version: "1".into(),
        config_generation: 3,
    }
}

fn lineage() -> LineageInput {
    LineageInput {
        inputs: Vec::new(),
        params_hash: "params".into(),
    }
}

fn ensure(identity: &str, alias: &str) -> Op {
    Op::Ensure {
        identity: identity.into(),
        ns: None,
        labels: BTreeMap::new(),
        r#as: alias.into(),
    }
}

fn text_facet(alias: &str, text: &str) -> Op {
    Op::PutFacet {
        target: Target::Alias(alias.into()),
        kind: FacetKind::Text,
        variant: None,
        body: Body::Text { text: text.into() },
        media_type: Some("text/plain".into()),
        lineage: lineage(),
    }
}

#[tokio::test]
async fn crash_replay_does_not_duplicate_rows() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::open(dir.path(), Options::default()).await.unwrap();
    engine
        .apply(
            ChangeSet {
                producer: producer(),
                idempotency_key: None,
                ack: None,
                ops: vec![ensure("ext:doi:10.1", "a"), text_facet("a", "hello")],
            },
            ApplyOpts::default(),
        )
        .await
        .unwrap();
    engine.testing_stop_before_snapshot().await.unwrap();
    drop(engine);

    let engine = Engine::open(dir.path(), Options::default()).await.unwrap();
    assert_eq!(engine.testing_count("resources").await.unwrap(), 0);
    let view = engine.view_latest().await.unwrap();
    assert!(view.resources.is_empty());
    engine.flush().await.unwrap();
    assert_eq!(engine.testing_count("resources").await.unwrap(), 1);
    assert_eq!(engine.testing_count("facets").await.unwrap(), 1);
    assert_eq!(engine.testing_count("identities").await.unwrap(), 1);
    engine.flush().await.unwrap();
    assert_eq!(engine.testing_count("resources").await.unwrap(), 1);
    assert_eq!(engine.testing_count("facets").await.unwrap(), 1);
    assert_eq!(engine.testing_count("identities").await.unwrap(), 1);
}

#[tokio::test]
async fn blob_merge_insert_keeps_one_row_per_hash() {
    let dir = tempfile::tempdir().unwrap();
    let options = Options {
        inline_text_max: 4 * 1024 * 1024,
        blob_inline_threshold: 16,
        blob_dedicated_threshold: 64,
        staging_ttl: Duration::from_secs(24 * 60 * 60),
    };
    let engine = Engine::open(dir.path(), options).await.unwrap();
    let small = engine.stage(&[7; 8]).await.unwrap();
    let middle = engine.stage(&[8; 32]).await.unwrap();
    let large = engine.stage(&[9; 96]).await.unwrap();
    let file = |alias: &str, part: strata_model::PartRef| Op::PutFacet {
        target: Target::Alias(alias.into()),
        kind: FacetKind::File,
        variant: None,
        body: Body::Staged { part },
        media_type: Some("application/octet-stream".into()),
        lineage: lineage(),
    };
    engine
        .apply(
            ChangeSet {
                producer: producer(),
                idempotency_key: None,
                ack: None,
                ops: vec![
                    ensure("ext:doi:blob-a", "a"),
                    ensure("ext:doi:blob-b", "b"),
                    ensure("ext:doi:blob-c", "c"),
                    file("a", small.clone()),
                    file("b", middle.clone()),
                    file("c", large.clone()),
                ],
            },
            ApplyOpts::default(),
        )
        .await
        .unwrap();
    engine.flush().await.unwrap();
    let small_again = engine.stage(&[7; 8]).await.unwrap();
    engine
        .apply(
            ChangeSet {
                producer: producer(),
                idempotency_key: None,
                ack: None,
                ops: vec![ensure("ext:doi:blob-d", "d"), file("d", small_again)],
            },
            ApplyOpts::default(),
        )
        .await
        .unwrap();
    engine.flush().await.unwrap();
    assert_eq!(engine.testing_count("blobs").await.unwrap(), 3);
    assert_eq!(
        engine.testing_blob(small.sha256()).await.unwrap(),
        vec![7; 8]
    );
    assert_eq!(
        engine.testing_blob(middle.sha256()).await.unwrap(),
        vec![8; 32]
    );
    assert_eq!(
        engine.testing_blob(large.sha256()).await.unwrap(),
        vec![9; 96]
    );
    let files = engine.testing_files().await.unwrap();
    eprintln!("blob layout files:");
    for file in &files {
        eprintln!("{file}");
    }
}

#[tokio::test]
async fn read_stays_on_observed_snapshot_after_rewind() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::open(dir.path(), Options::default()).await.unwrap();
    engine
        .apply(
            ChangeSet {
                producer: producer(),
                idempotency_key: None,
                ack: None,
                ops: vec![ensure("ext:doi:mono-a", "a"), text_facet("a", "A")],
            },
            ApplyOpts::default(),
        )
        .await
        .unwrap();
    engine.flush().await.unwrap();
    engine
        .apply(
            ChangeSet {
                producer: producer(),
                idempotency_key: None,
                ack: None,
                ops: vec![ensure("ext:doi:mono-b", "b"), text_facet("b", "B")],
            },
            ApplyOpts::default(),
        )
        .await
        .unwrap();
    engine.flush().await.unwrap();
    let seen = engine.view_latest().await.unwrap();
    assert_eq!(seen.snapshot.unwrap().seq, 2);
    assert_eq!(seen.resources.len(), 2);
    engine.testing_rewind_to(1).await.unwrap();
    assert_eq!(engine.testing_count("resources").await.unwrap(), 1);
    let still = engine.view_latest().await.unwrap();
    assert_eq!(still.snapshot.unwrap().seq, 2);
    assert_eq!(still.resources.len(), 2);
    assert_eq!(still.facets.len(), 2);
}

#[tokio::test]
async fn empty_flush_sweeps_expired_staging() {
    let dir = tempfile::tempdir().unwrap();
    let options = Options {
        staging_ttl: Duration::ZERO,
        ..Options::default()
    };
    let engine = Engine::open(dir.path(), options).await.unwrap();
    let part = engine.stage(b"orphan").await.unwrap();
    let path = dir.path().join("staging").join(part.sha256());
    assert!(path.is_file());
    assert!(engine.flush().await.unwrap().is_none());
    assert!(!path.exists());
}

#[tokio::test]
async fn visible_timeout_leaves_flush_running() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::open(dir.path(), Options::default()).await.unwrap();
    let hold = Arc::new(tokio::sync::Notify::new());
    let done = Arc::new(tokio::sync::Notify::new());
    engine
        .testing_pause_flush(Arc::clone(&hold), Arc::clone(&done))
        .await;
    let error = engine
        .apply(
            ChangeSet {
                producer: producer(),
                idempotency_key: None,
                ack: None,
                ops: vec![
                    ensure("ext:doi:visible-wait", "a"),
                    text_facet("a", "later"),
                ],
            },
            ApplyOpts {
                wait: WaitFor::Visible,
                timeout: Some(Duration::from_millis(200)),
            },
        )
        .await
        .unwrap_err();
    let crate::Error::WaitTimeout(commit_id) = error else {
        panic!("期望等待超时，得到 {error}");
    };
    assert!(!commit_id.is_empty());
    assert!(engine.view_latest().await.unwrap().resources.is_empty());
    hold.notify_one();
    done.notified().await;
    let view = engine.view_latest().await.unwrap();
    assert_eq!(view.resources.len(), 1);
}
