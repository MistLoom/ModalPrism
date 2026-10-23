//! S3 / RustFS 后端集成验证。默认忽略，配置实例后手动运行：
//!
//! `STRATA_TEST_S3_BUCKET=media STRATA_TEST_S3_ENDPOINT=http://127.0.0.1:9000 \
//!  cargo test -p strata-gateway --features s3 --test s3 -- --ignored`
//!
//! 可选变量：`STRATA_TEST_S3_REGION`、`STRATA_TEST_S3_PREFIX`。凭据默认沿用
//! `AmazonS3Builder::from_env()` 的 AWS 变量；也可以把
//! `STRATA_TEST_S3_ACCESS_KEY_ENV` / `STRATA_TEST_S3_SECRET_KEY_ENV` 设为保存
//! 凭据的环境变量名，由数据层按名字读取，不把密钥写进测试。

#![cfg(feature = "s3")]

use std::time::{SystemTime, UNIX_EPOCH};

use strata_gateway::BlobBackend;
use strata_gateway::blob;

fn backend(prefix: String) -> BlobBackend {
    let bucket =
        std::env::var("STRATA_TEST_S3_BUCKET").expect("需要 STRATA_TEST_S3_BUCKET 指向已存在的桶");
    BlobBackend::S3 {
        bucket,
        region: std::env::var("STRATA_TEST_S3_REGION").ok(),
        endpoint: std::env::var("STRATA_TEST_S3_ENDPOINT").ok(),
        prefix: Some(prefix),
        access_key_env: std::env::var("STRATA_TEST_S3_ACCESS_KEY_ENV").ok(),
        secret_key_env: std::env::var("STRATA_TEST_S3_SECRET_KEY_ENV").ok(),
    }
}

#[tokio::test]
#[ignore = "需要 STRATA_TEST_S3_BUCKET 等环境变量"]
async fn s3_roundtrip_ranges_and_prefix_isolation() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let base = std::env::var("STRATA_TEST_S3_PREFIX").unwrap_or_else(|_| "strata-it".into());
    let root = format!("{base}/{}-{stamp}", std::process::id());
    let dir = tempfile::tempdir().unwrap();
    let store_a = blob::open_store(&backend(format!("{root}/a")), dir.path())
        .await
        .unwrap();
    let store_b = blob::open_store(&backend(format!("{root}/b")), dir.path())
        .await
        .unwrap();

    let bytes = b"content addressed range read ".repeat(4_000);
    let sha = strata_model::content_hash(&bytes);
    store_a.put(&sha, &bytes).await.unwrap();
    assert_eq!(store_a.get(&sha).await.unwrap(), bytes);
    assert_eq!(store_a.range(&sha, 3, 9).await.unwrap(), &bytes[3..12]);
    assert!(store_a.contains(&sha).await.unwrap());

    // 同一对象重复写入是幂等的；长度不符则必须报错，不能静默接受。
    store_a.put(&sha, &bytes).await.unwrap();
    let shorter = &bytes[..bytes.len() - 1];
    assert!(matches!(
        store_a.put(&sha, shorter).await,
        Err(strata_gateway::Error::BlobMismatch(_))
    ));

    // 前缀隔离：两个前缀下同一个 sha 互不影响。
    store_b.put(&sha, &bytes).await.unwrap();
    store_a.remove(&sha).await.unwrap();
    assert!(!store_a.contains(&sha).await.unwrap());
    assert_eq!(store_b.get(&sha).await.unwrap(), bytes);

    store_b.remove(&sha).await.unwrap();
}
