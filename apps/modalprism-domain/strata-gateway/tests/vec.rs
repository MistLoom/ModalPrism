//! 本地 SQLite 后端默认带 sqlite-vec：打开即注册，向量检索走 `vec_distance_cosine`。

use std::sync::Arc;

use strata_gateway::{
    Backend, Db, Dialect, Gateway, IndexStore, Options, Sql, Tokenizer, VectorQuery,
};

#[tokio::test]
async fn opens_with_sqlite_vec_registered() {
    let dir = tempfile::tempdir().unwrap();
    let gateway = Gateway::open(dir.path(), Options::default()).await.unwrap();
    assert!(gateway.vector_accelerated(), "sqlite-vec 没有注册");
}

#[tokio::test]
async fn vector_search_uses_the_registered_extension() {
    let dir = tempfile::tempdir().unwrap();
    // 先让 Gateway 建表，再用同一文件的第二个连接验证索引路径。
    let gateway = Gateway::open(dir.path(), Options::default()).await.unwrap();
    assert!(gateway.vector_accelerated(), "sqlite-vec 没有注册");

    let db = Arc::new(
        Db::open(&Backend::default(), dir.path())
            .await
            .expect("打开同一个 gateway.db"),
    );
    for (rid, vector) in [("ext:a", [1.0f32, 0.0, 0.0]), ("ext:b", [0.0f32, 1.0, 0.0])] {
        let mut sql = Sql::new(Dialect::Sqlite);
        sql.push("INSERT INTO g_vectors (rid, kind, variant, model, dims, vec, v) VALUES (");
        sql.text(rid);
        sql.push(", ").text("embedding");
        sql.push(", ").text("demo");
        sql.push(", ").text("demo-v1");
        sql.push(", ").int(3);
        sql.push(", ");
        sql.bytes(
            vector
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect(),
        );
        sql.push(", ").int(1);
        sql.push(")");
        db.exec(&sql).await.unwrap();
    }

    let index = IndexStore::open(Arc::clone(&db), Tokenizer::Unicode)
        .await
        .unwrap();
    assert!(index.sqlite_vec(), "索引没有走 sqlite-vec");

    let scores = index
        .search_vectors(&VectorQuery {
            vector: &[1.0, 0.0, 0.0],
            k: 2,
            kind: Some("embedding"),
            variant: Some("demo"),
            model: None,
            version: 1,
        })
        .await
        .unwrap();
    assert_eq!(
        scores.first().map(|score| score.rid.as_str()),
        Some("ext:a"),
        "最近的向量应排在最前: {scores:?}"
    );
}
