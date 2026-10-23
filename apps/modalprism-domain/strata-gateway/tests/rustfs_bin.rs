//! 外部 RustFS 二进制：真实下载官方发行包、起子进程、走完整的读写。
//!
//! 首次会下载约 110 MB，所以默认不跑：
//! `cargo test -p strata-gateway --test rustfs_bin -- --ignored`

#![cfg(feature = "rustfs-bin")]

use std::path::PathBuf;

use strata_gateway::{BlobBackend, blob};

fn backend() -> BlobBackend {
    BlobBackend::Rustfs {
        exe: None,
        cache: None,
        offline: false,
        bucket: None,
        prefix: None,
        path: None,
    }
}

/// 设置 `STRATA_RUSTFS_DATA` 时保留数据目录与日志，便于排查。
fn data_root() -> (Option<tempfile::TempDir>, PathBuf) {
    match std::env::var_os("STRATA_RUSTFS_DATA") {
        Some(path) => (None, PathBuf::from(path)),
        None => {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().to_path_buf();
            (Some(dir), root)
        }
    }
}

#[tokio::test]
#[ignore = "需要下载 RustFS 发行包"]
async fn downloads_the_release_and_roundtrips() {
    let (_guard, dir) = data_root();
    let store = blob::open_store(&backend(), &dir).await.unwrap();

    let sha = "a".repeat(64);
    store.put(&sha, b"hello rustfs").await.unwrap();
    assert!(store.contains(&sha).await.unwrap());
    assert_eq!(store.get(&sha).await.unwrap(), b"hello rustfs");
    assert_eq!(store.range(&sha, 6, 6).await.unwrap(), b"rustfs");
    store.remove(&sha).await.unwrap();
    assert!(!store.contains(&sha).await.unwrap());

    // 缓存目录与数据卷都落在数据根下。
    assert!(dir.join("tools/rustfs/1.0.3").is_dir());
    assert!(dir.join("rustfs").is_dir());
}

#[tokio::test]
#[ignore = "需要下载 RustFS 发行包"]
async fn a_second_open_reuses_the_cached_binary() {
    let (_guard, dir) = data_root();
    {
        let store = blob::open_store(&backend(), &dir).await.unwrap();
        let sha = "b".repeat(64);
        store.put(&sha, b"persisted").await.unwrap();
    }
    let store = blob::open_store(&backend(), &dir).await.unwrap();
    assert_eq!(store.get(&"b".repeat(64)).await.unwrap(), b"persisted");
}
