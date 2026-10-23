//! 物理 schema。逻辑表名沿用 Strata 既有语义，物理名加 `g_` 前缀。
//!
//! 每张数据表都有两列版本信息：`seq` 是全局写入序号，`v` 是表内写入版本。
//! 快照钉住各表版本，读取时用 `v <= 快照版本` 还原当时可见的行。

use crate::config::Tokenizer;
use crate::db::Db;
#[cfg(feature = "postgres")]
use crate::error::Error;
use crate::error::Result;
use crate::sql::{Dialect, Sql};

/// 参与快照的逻辑表。
pub const LOGICAL_TABLES: [&str; 10] = [
    "resources",
    "resource_marks",
    "facets",
    "blobs",
    "relations",
    "identities",
    "state",
    "snapshots",
    "vectors",
    "text",
];

pub fn physical(logical: &str) -> String {
    format!("g_{logical}")
}

fn common_ddl(dialect: Dialect) -> Vec<String> {
    let blob = dialect.blob_type();
    vec![
        "CREATE TABLE IF NOT EXISTS g_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)".into(),
        "CREATE TABLE IF NOT EXISTS g_counters (key TEXT PRIMARY KEY, value BIGINT NOT NULL)"
            .into(),
        "CREATE TABLE IF NOT EXISTS g_versions (name TEXT PRIMARY KEY, version BIGINT NOT NULL)"
            .into(),
        "CREATE TABLE IF NOT EXISTS g_resources (
            rid TEXT NOT NULL,
            ns TEXT NOT NULL,
            source_id TEXT,
            created_at TEXT NOT NULL,
            seq BIGINT NOT NULL,
            v BIGINT NOT NULL
        )"
        .into(),
        "CREATE TABLE IF NOT EXISTS g_resource_marks (
            rid TEXT NOT NULL,
            labels TEXT NOT NULL,
            tombstone BIGINT NOT NULL,
            reason TEXT,
            seq BIGINT NOT NULL,
            v BIGINT NOT NULL
        )"
        .into(),
        "CREATE TABLE IF NOT EXISTS g_facets (
            rid TEXT NOT NULL,
            kind TEXT NOT NULL,
            variant TEXT NOT NULL DEFAULT '',
            version BIGINT NOT NULL,
            content_hash TEXT NOT NULL,
            media_type TEXT,
            size BIGINT NOT NULL,
            body_text TEXT,
            body_struct TEXT,
            blob_ref TEXT,
            inputs TEXT NOT NULL,
            params_hash TEXT NOT NULL,
            producer_module TEXT NOT NULL,
            producer_version TEXT NOT NULL,
            config_generation BIGINT NOT NULL,
            commit_id TEXT NOT NULL,
            created_at TEXT NOT NULL,
            seq BIGINT NOT NULL,
            v BIGINT NOT NULL
        )"
        .into(),
        format!(
            "CREATE TABLE IF NOT EXISTS g_blobs (
            sha256 TEXT PRIMARY KEY,
            size BIGINT NOT NULL,
            media_type TEXT,
            tier TEXT NOT NULL,
            inline {blob},
            created_at TEXT NOT NULL,
            seq BIGINT NOT NULL,
            v BIGINT NOT NULL
        )"
        ),
        "CREATE TABLE IF NOT EXISTS g_relations (
            src_rid TEXT NOT NULL,
            rel TEXT NOT NULL,
            dst_rid TEXT NOT NULL,
            attrs TEXT,
            commit_id TEXT NOT NULL,
            seq BIGINT NOT NULL,
            v BIGINT NOT NULL
        )"
        .into(),
        "CREATE TABLE IF NOT EXISTS g_identities (
            identity_key TEXT PRIMARY KEY,
            rid TEXT NOT NULL,
            scheme TEXT NOT NULL,
            commit_id TEXT NOT NULL,
            seq BIGINT NOT NULL,
            v BIGINT NOT NULL
        )"
        .into(),
        "CREATE TABLE IF NOT EXISTS g_state (
            scope TEXT NOT NULL,
            key TEXT NOT NULL,
            value TEXT NOT NULL,
            commit_id TEXT NOT NULL,
            seq BIGINT NOT NULL,
            v BIGINT NOT NULL
        )"
        .into(),
        "CREATE TABLE IF NOT EXISTS g_snapshots (
            snapshot_seq BIGINT PRIMARY KEY,
            table_versions TEXT NOT NULL,
            commit_ids TEXT NOT NULL,
            kind TEXT NOT NULL,
            config_generation BIGINT NOT NULL,
            published_at TEXT NOT NULL,
            v BIGINT NOT NULL
        )"
        .into(),
        "CREATE TABLE IF NOT EXISTS g_commits (
            commit_id TEXT PRIMARY KEY,
            idempotency_key TEXT UNIQUE,
            producer_module TEXT NOT NULL,
            producer_version TEXT NOT NULL,
            config_generation BIGINT NOT NULL,
            snapshot_seq BIGINT,
            status TEXT NOT NULL,
            staging TEXT NOT NULL,
            created_at TEXT NOT NULL
        )"
        .into(),
        "CREATE TABLE IF NOT EXISTS g_tasks (
            task_id TEXT PRIMARY KEY,
            queue TEXT NOT NULL,
            payload TEXT NOT NULL,
            status TEXT NOT NULL,
            lease_until TEXT,
            retry_at TEXT,
            last_error TEXT,
            priority BIGINT NOT NULL,
            created_at TEXT NOT NULL,
            source_id TEXT NOT NULL DEFAULT '',
            locator_key TEXT NOT NULL DEFAULT '',
            attempt BIGINT NOT NULL DEFAULT 0,
            generation BIGINT NOT NULL DEFAULT 0,
            reason TEXT NOT NULL DEFAULT ''
        )"
        .into(),
        "CREATE TABLE IF NOT EXISTS g_hot (
            kind TEXT NOT NULL,
            key TEXT NOT NULL,
            value TEXT NOT NULL,
            PRIMARY KEY (kind, key)
        )"
        .into(),
        // 索引
        "CREATE INDEX IF NOT EXISTS g_resources_v ON g_resources(v)".into(),
        "CREATE INDEX IF NOT EXISTS g_resources_rid ON g_resources(rid)".into(),
        "CREATE INDEX IF NOT EXISTS g_resource_marks_rid ON g_resource_marks(rid, seq)".into(),
        "CREATE INDEX IF NOT EXISTS g_resource_marks_v ON g_resource_marks(v)".into(),
        "CREATE INDEX IF NOT EXISTS g_facets_head ON g_facets(rid, kind, variant, version)".into(),
        "CREATE INDEX IF NOT EXISTS g_facets_kind ON g_facets(kind, variant)".into(),
        "CREATE INDEX IF NOT EXISTS g_facets_v ON g_facets(v)".into(),
        "CREATE INDEX IF NOT EXISTS g_relations_src ON g_relations(src_rid, rel)".into(),
        "CREATE INDEX IF NOT EXISTS g_relations_dst ON g_relations(dst_rid, rel)".into(),
        "CREATE INDEX IF NOT EXISTS g_relations_v ON g_relations(v)".into(),
        "CREATE INDEX IF NOT EXISTS g_state_key ON g_state(scope, key, v)".into(),
        "CREATE INDEX IF NOT EXISTS g_snapshots_v ON g_snapshots(v)".into(),
        "CREATE INDEX IF NOT EXISTS g_tasks_queue ON g_tasks(queue, status)".into(),
        "CREATE INDEX IF NOT EXISTS g_tasks_locator ON g_tasks(source_id, locator_key, status)"
            .into(),
        "CREATE INDEX IF NOT EXISTS g_commits_status ON g_commits(status, created_at)".into(),
        "CREATE INDEX IF NOT EXISTS g_commits_snapshot ON g_commits(snapshot_seq)".into(),
        "CREATE INDEX IF NOT EXISTS g_blobs_v ON g_blobs(v)".into(),
    ]
}

fn sqlite_ddl(tokenizer: Tokenizer) -> Vec<String> {
    vec![
        // 精确检索的规范表：向量以 f32 LE 字节串存放，可跨版本回溯。
        "CREATE TABLE IF NOT EXISTS g_vectors (
            rid TEXT NOT NULL,
            kind TEXT NOT NULL,
            variant TEXT NOT NULL DEFAULT '',
            model TEXT NOT NULL DEFAULT '',
            dims BIGINT NOT NULL,
            vec BLOB NOT NULL,
            v BIGINT NOT NULL,
            PRIMARY KEY (rid, kind, variant, model, v)
        )"
        .into(),
        "CREATE INDEX IF NOT EXISTS g_vectors_scope ON g_vectors(kind, variant, model, v)".into(),
        format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS g_fts USING fts5(
            rid, kind, variant, body, v UNINDEXED, tokenize='{}'
        )",
            tokenizer.fts5_name()
        ),
    ]
}

#[cfg(feature = "postgres")]
fn postgres_ddl() -> Vec<String> {
    vec![
        // pgvector 表按维度在首次写入时创建；扩展缺失只影响向量能力。
        "CREATE TABLE IF NOT EXISTS g_docs (
            rid TEXT NOT NULL,
            kind TEXT NOT NULL,
            variant TEXT NOT NULL DEFAULT '',
            body TEXT NOT NULL,
            v BIGINT NOT NULL
        )"
        .into(),
        "CREATE INDEX IF NOT EXISTS g_docs_fts ON g_docs USING GIN (to_tsvector('simple', body))"
            .into(),
        "CREATE INDEX IF NOT EXISTS g_docs_v ON g_docs(v)".into(),
    ]
}

/// 有 pgvector 就启用扩展；服务端没装该扩展时跳过，文本与行数据照常迁移。
#[cfg(feature = "postgres")]
async fn ensure_vector_extension(db: &Db) -> Result<()> {
    let mut installed = Sql::new(Dialect::Postgres);
    installed.push("SELECT count(*) FROM pg_extension WHERE extname = 'vector'");
    if db.fetch_int(&installed).await?.unwrap_or(0) > 0 {
        return Ok(());
    }
    let mut available = Sql::new(Dialect::Postgres);
    available.push("SELECT count(*) FROM pg_available_extensions WHERE name = 'vector'");
    if db.fetch_int(&available).await?.unwrap_or(0) == 0 {
        return Ok(());
    }
    let mut create = Sql::new(Dialect::Postgres);
    create.push("CREATE EXTENSION IF NOT EXISTS vector");
    match db.exec(&create).await {
        Ok(_) => Ok(()),
        // 没有安装权限或文件缺失时留给向量读写暴露具体错误，不阻断其余表。
        Err(Error::Sql { source, .. })
            if source
                .as_database_error()
                .and_then(|error| error.code())
                .is_some_and(|code| matches!(code.as_ref(), "0A000" | "42501" | "58P01")) =>
        {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

/// 建表并补齐版本计数器。可重复调用。
pub async fn migrate(db: &Db, tokenizer: Tokenizer) -> Result<()> {
    let dialect = db.dialect();
    let mut statements = common_ddl(dialect);
    match dialect {
        Dialect::Sqlite => statements.extend(sqlite_ddl(tokenizer)),
        Dialect::Postgres => {
            #[cfg(feature = "postgres")]
            {
                ensure_vector_extension(db).await?;
                statements.extend(postgres_ddl());
            }
            #[cfg(not(feature = "postgres"))]
            return Err(crate::error::Error::BackendDisabled("postgres"));
        }
    }
    let mut prepared: Vec<Sql> = statements
        .iter()
        .map(|statement| Sql::raw(dialect, statement))
        .collect();
    for logical in LOGICAL_TABLES {
        let mut sql = Sql::new(dialect);
        sql.push("INSERT INTO g_versions(name, version) VALUES (");
        sql.text(logical);
        sql.push(", 0) ON CONFLICT(name) DO NOTHING");
        prepared.push(sql);
    }
    db.exec_batch(&prepared).await
}

/// 计数器原子累加，返回累加后的值。首次调用从 0 开始。
pub async fn alloc_counter(db: &Db, key: &str, count: u64) -> Result<u64> {
    let mut tx = db.begin().await?;
    let value = alloc_counter_tx(&mut tx, key, count).await?;
    tx.commit().await?;
    Ok(value)
}

/// 事务内累加计数器，与数据写入保持同一原子单元。
pub async fn alloc_counter_tx(tx: &mut crate::db::Tx<'_>, key: &str, count: u64) -> Result<u64> {
    let dialect = tx.dialect();
    let mut init = Sql::new(dialect);
    init.push("INSERT INTO g_counters(key, value) VALUES (");
    init.text(key);
    init.push(", 0) ON CONFLICT(key) DO NOTHING");
    tx.exec(&init).await?;
    let mut bump = Sql::new(dialect);
    bump.push("UPDATE g_counters SET value = value + ");
    bump.u64(count.max(1));
    bump.push(" WHERE key = ");
    bump.text(key);
    bump.push(" RETURNING value");
    let value = tx
        .fetch_int(&bump)
        .await?
        .ok_or_else(|| crate::error::Error::InvalidConfig(format!("计数器缺失: {key}")))?;
    Ok(value.max(0) as u64)
}
