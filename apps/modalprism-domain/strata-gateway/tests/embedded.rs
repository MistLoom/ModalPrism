//! 进程内 RustFS：起服务、建桶、读写与区间读都在同一个进程里完成。

#![cfg(feature = "embedded-s3")]

use strata_gateway::{BlobBackend, Gateway, Options, blob};

fn backend() -> BlobBackend {
    BlobBackend::Embedded {
        bucket: None,
        prefix: None,
        path: None,
    }
}

#[tokio::test]
async fn embedded_rustfs_roundtrips_and_ranges() {
    let dir = tempfile::tempdir().unwrap();
    let store = blob::open_store(&backend(), dir.path()).await.unwrap();
    assert!(
        dir.path().join("rustfs").is_dir(),
        "数据卷应当落在数据目录下"
    );

    let bytes = b"embedded rustfs object payload";
    let sha = strata_model::content_hash(bytes);
    store.put(&sha, bytes).await.unwrap();
    assert!(store.contains(&sha).await.unwrap());
    assert_eq!(store.get(&sha).await.unwrap(), bytes);
    assert_eq!(store.range(&sha, 9, 6).await.unwrap(), b"rustfs");

    // 重复写同一内容按长度核对，不报错。
    store.put(&sha, bytes).await.unwrap();

    store.remove(&sha).await.unwrap();
    assert!(!store.contains(&sha).await.unwrap());
}

#[tokio::test]
async fn gateway_open_creates_the_rustfs_volume() {
    let dir = tempfile::tempdir().unwrap();
    let gateway = Gateway::open(
        dir.path(),
        Options {
            blobs: backend(),
            ..Options::default()
        },
    )
    .await
    .unwrap();
    assert!(dir.path().join("gateway.db").is_file());
    assert!(dir.path().join("rustfs").is_dir());
    assert_eq!(gateway.backend_names().1, "embedded");
    assert!(
        gateway.vector_accelerated(),
        "嵌入式构建同样默认带 sqlite-vec"
    );
}

#[tokio::test]
async fn two_embedded_servers_coexist_in_one_process() {
    // 多个实例共存是 rustfs 的明确承诺；这里顺带确认端口与数据卷互不干扰。
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let one = blob::open_store(&backend(), first.path()).await.unwrap();
    let two = blob::open_store(&backend(), second.path()).await.unwrap();

    let a = b"first volume";
    let b = b"second volume";
    let (sha_a, sha_b) = (strata_model::content_hash(a), strata_model::content_hash(b));
    one.put(&sha_a, a).await.unwrap();
    two.put(&sha_b, b).await.unwrap();
    assert_eq!(one.get(&sha_a).await.unwrap(), a);
    assert_eq!(two.get(&sha_b).await.unwrap(), b);
    assert!(!one.contains(&sha_b).await.unwrap(), "两个实例的桶应当隔离");
    assert!(!two.contains(&sha_a).await.unwrap());
}
