//! 版本化数据平面。
//!
//! 写入按表递增版本号，行上打 `v`；快照记录各表版本。读取用 `v <= 快照版本`
//! 还原当时可见的行，等价于不可变版本的语义，但只依赖普通 SQL。

use std::collections::BTreeMap;

use serde_json::Value;

use crate::blob::BlobTier;
use crate::db::{Db, Row as DbRow, Tx};
use crate::error::{Error, Result};
use crate::schema::{LOGICAL_TABLES, alloc_counter_tx, physical};
use crate::sql::{Bind, Dialect, Sql};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResourceRow {
    pub rid: String,
    pub ns: String,
    pub source_id: Option<String>,
    pub created_at: String,
    pub seq: u64,
    pub v: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MarkRow {
    pub rid: String,
    pub labels: Vec<String>,
    pub tombstone: bool,
    pub reason: Option<String>,
    pub seq: u64,
    pub v: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FacetRow {
    pub rid: String,
    pub kind: String,
    pub variant: Option<String>,
    pub version: u64,
    pub content_hash: String,
    pub media_type: Option<String>,
    pub size: u64,
    pub body_text: Option<String>,
    pub body_struct: Option<String>,
    pub blob_ref: Option<String>,
    pub inputs: Vec<String>,
    pub params_hash: String,
    pub producer_module: String,
    pub producer_version: String,
    pub config_generation: u64,
    pub commit_id: String,
    pub created_at: String,
    pub seq: u64,
    pub v: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlobRow {
    pub sha256: String,
    pub size: u64,
    pub media_type: Option<String>,
    pub tier: BlobTier,
    pub inline: Option<Vec<u8>>,
    pub created_at: String,
    pub seq: u64,
    pub v: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RelationRow {
    pub src_rid: String,
    pub rel: String,
    pub dst_rid: String,
    pub attrs: Option<Value>,
    pub commit_id: String,
    pub seq: u64,
    pub v: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IdentityRow {
    pub identity_key: String,
    pub rid: String,
    pub scheme: String,
    pub commit_id: String,
    pub seq: u64,
    pub v: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StateRow {
    pub scope: String,
    pub key: String,
    pub value: Value,
    pub commit_id: String,
    pub seq: u64,
    pub v: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SnapshotRow {
    pub seq: u64,
    pub versions: BTreeMap<String, u64>,
    pub commit_ids: Vec<String>,
    pub kind: String,
    pub config_generation: u64,
    pub published_at: String,
    pub v: u64,
}

/// 一条待写入的向量。`variant` 为空串表示无 variant。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VectorRow {
    pub rid: String,
    pub kind: String,
    pub variant: Option<String>,
    pub model: String,
    pub vector: Vec<f32>,
    pub v: u64,
}

/// 一条待写入的全文行。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextRow {
    pub rid: String,
    pub kind: String,
    pub variant: Option<String>,
    pub body: String,
    pub v: u64,
}

/// 一次刷写的数据。空组不会产生版本号。
#[derive(Clone, Debug, Default)]
pub struct Batch {
    pub resources: Vec<ResourceRow>,
    pub marks: Vec<MarkRow>,
    pub facets: Vec<FacetRow>,
    pub blobs: Vec<BlobRow>,
    pub relations: Vec<RelationRow>,
    pub identities: Vec<IdentityRow>,
    pub states: Vec<StateRow>,
    pub vectors: Vec<VectorRow>,
    pub texts: Vec<TextRow>,
}

impl Batch {
    pub fn is_empty(&self) -> bool {
        self.resources.is_empty()
            && self.marks.is_empty()
            && self.facets.is_empty()
            && self.blobs.is_empty()
            && self.relations.is_empty()
            && self.identities.is_empty()
            && self.states.is_empty()
            && self.vectors.is_empty()
            && self.texts.is_empty()
    }
}

/// 某个快照可见的全部行。
#[derive(Clone, Debug, Default)]
pub struct Dataset {
    pub snapshot: Option<SnapshotRow>,
    pub resources: Vec<ResourceRow>,
    pub marks: Vec<MarkRow>,
    pub facets: Vec<FacetRow>,
    pub relations: Vec<RelationRow>,
    pub identities: Vec<IdentityRow>,
    pub states: Vec<StateRow>,
}

fn version_of(versions: &BTreeMap<String, u64>, name: &str) -> u64 {
    versions.get(name).copied().unwrap_or(0)
}

async fn insert_row(
    tx: &mut Tx<'_>,
    dialect: Dialect,
    table: &str,
    columns: &str,
    binds: Vec<Bind>,
) -> Result<()> {
    insert_row_conflict(tx, dialect, table, columns, binds, None).await
}

async fn insert_row_conflict(
    tx: &mut Tx<'_>,
    dialect: Dialect,
    table: &str,
    columns: &str,
    binds: Vec<Bind>,
    conflict: Option<&str>,
) -> Result<()> {
    let mut sql = Sql::new(dialect);
    sql.push("INSERT INTO ").push(table).push(" (").push(columns);
    sql.push(") VALUES (");
    for (index, bind) in binds.into_iter().enumerate() {
        if index > 0 {
            sql.push(", ");
        }
        sql.bind(bind);
    }
    sql.push(")");
    if let Some(target) = conflict {
        sql.push(" ").push(dialect.on_conflict_do_nothing(target));
    }
    tx.exec(&sql).await?;
    Ok(())
}

async fn bump_version(tx: &mut Tx<'_>, dialect: Dialect, logical: &str) -> Result<u64> {
    let mut sql = Sql::new(dialect);
    sql.push("INSERT INTO g_versions(name, version) VALUES (");
    sql.text(logical);
    sql.push(", 1) ON CONFLICT(name) DO UPDATE SET version = g_versions.version + 1 RETURNING version");
    let value = tx.fetch_int(&sql).await?.ok_or_else(|| {
        Error::InvalidConfig(format!("版本计数器缺失: {logical}"))
    })?;
    Ok(value.max(0) as u64)
}

fn vec_bytes(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len() * 4);
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn vec_text(vector: &[f32]) -> String {
    let mut text = String::from("[");
    for (index, value) in vector.iter().enumerate() {
        if index > 0 {
            text.push(',');
        }
        text.push_str(&value.to_string());
    }
    text.push(']');
    text
}

/// 写入一批行与索引，返回本次递增的版本。
pub async fn write(db: &Db, batch: &mut Batch) -> Result<BTreeMap<String, u64>> {
    let dialect = db.dialect();
    let mut tx = db.begin().await?;
    let mut versions = BTreeMap::new();

    for (logical, filled) in [
        ("resources", !batch.resources.is_empty()),
        ("resource_marks", !batch.marks.is_empty()),
        ("facets", !batch.facets.is_empty()),
        ("blobs", !batch.blobs.is_empty()),
        ("relations", !batch.relations.is_empty()),
        ("identities", !batch.identities.is_empty()),
        ("state", !batch.states.is_empty()),
        ("vectors", !batch.vectors.is_empty()),
        ("text", !batch.texts.is_empty()),
    ] {
        if filled {
            versions.insert(logical.to_string(), bump_version(&mut tx, dialect, logical).await?);
        }
    }

    let resources_v = version_of(&versions, "resources");
    let marks_v = version_of(&versions, "resource_marks");
    let facets_v = version_of(&versions, "facets");
    let blobs_v = version_of(&versions, "blobs");
    let relations_v = version_of(&versions, "relations");
    let identities_v = version_of(&versions, "identities");
    let states_v = version_of(&versions, "state");
    let vectors_v = version_of(&versions, "vectors");
    let texts_v = version_of(&versions, "text");

    for row in &mut batch.resources {
        row.v = resources_v;
    }
    for row in &mut batch.marks {
        row.v = marks_v;
    }
    for row in &mut batch.facets {
        row.v = facets_v;
    }
    for row in &mut batch.blobs {
        row.v = blobs_v;
    }
    for row in &mut batch.relations {
        row.v = relations_v;
    }
    for row in &mut batch.identities {
        row.v = identities_v;
    }
    for row in &mut batch.states {
        row.v = states_v;
    }
    for row in &mut batch.vectors {
        row.v = vectors_v;
    }
    for row in &mut batch.texts {
        row.v = texts_v;
    }

    for row in &batch.resources {
        insert_row(
            &mut tx,
            dialect,
            "g_resources",
            "rid, ns, source_id, created_at, seq, v",
            vec![
                Bind::Text(row.rid.clone()),
                Bind::Text(row.ns.clone()),
                Bind::OptText(row.source_id.clone()),
                Bind::Text(row.created_at.clone()),
                Bind::Int(row.seq as i64),
                Bind::Int(row.v as i64),
            ],
        )
        .await?;
    }
    for row in &batch.marks {
        insert_row(
            &mut tx,
            dialect,
            "g_resource_marks",
            "rid, labels, tombstone, reason, seq, v",
            vec![
                Bind::Text(row.rid.clone()),
                Bind::Text(serde_json::to_string(&row.labels)?),
                Bind::Int(if row.tombstone { 1 } else { 0 }),
                Bind::OptText(row.reason.clone()),
                Bind::Int(row.seq as i64),
                Bind::Int(row.v as i64),
            ],
        )
        .await?;
    }
    for row in &batch.facets {
        insert_row(
            &mut tx,
            dialect,
            "g_facets",
            "rid, kind, variant, version, content_hash, media_type, size, body_text, body_struct, blob_ref, inputs, params_hash, producer_module, producer_version, config_generation, commit_id, created_at, seq, v",
            vec![
                Bind::Text(row.rid.clone()),
                Bind::Text(row.kind.clone()),
                Bind::Text(row.variant.clone().unwrap_or_default()),
                Bind::Int(row.version as i64),
                Bind::Text(row.content_hash.clone()),
                Bind::OptText(row.media_type.clone()),
                Bind::Int(row.size as i64),
                Bind::OptText(row.body_text.clone()),
                Bind::OptText(row.body_struct.clone()),
                Bind::OptText(row.blob_ref.clone()),
                Bind::Text(serde_json::to_string(&row.inputs)?),
                Bind::Text(row.params_hash.clone()),
                Bind::Text(row.producer_module.clone()),
                Bind::Text(row.producer_version.clone()),
                Bind::Int(row.config_generation as i64),
                Bind::Text(row.commit_id.clone()),
                Bind::Text(row.created_at.clone()),
                Bind::Int(row.seq as i64),
                Bind::Int(row.v as i64),
            ],
        )
        .await?;
    }
    for row in &batch.blobs {
        insert_row_conflict(
            &mut tx,
            dialect,
            "g_blobs",
            "sha256, size, media_type, tier, inline, created_at, seq, v",
            vec![
                Bind::Text(row.sha256.clone()),
                Bind::Int(row.size as i64),
                Bind::OptText(row.media_type.clone()),
                Bind::Text(row.tier.as_str().to_string()),
                Bind::OptBytes(row.inline.clone()),
                Bind::Text(row.created_at.clone()),
                Bind::Int(row.seq as i64),
                Bind::Int(row.v as i64),
            ],
            Some("sha256"),
        )
        .await?;
    }
    for row in &batch.relations {
        insert_row(
            &mut tx,
            dialect,
            "g_relations",
            "src_rid, rel, dst_rid, attrs, commit_id, seq, v",
            vec![
                Bind::Text(row.src_rid.clone()),
                Bind::Text(row.rel.clone()),
                Bind::Text(row.dst_rid.clone()),
                Bind::OptText(row.attrs.as_ref().map(serde_json::to_string).transpose()?),
                Bind::Text(row.commit_id.clone()),
                Bind::Int(row.seq as i64),
                Bind::Int(row.v as i64),
            ],
        )
        .await?;
    }
    for row in &batch.identities {
        insert_row_conflict(
            &mut tx,
            dialect,
            "g_identities",
            "identity_key, rid, scheme, commit_id, seq, v",
            vec![
                Bind::Text(row.identity_key.clone()),
                Bind::Text(row.rid.clone()),
                Bind::Text(row.scheme.clone()),
                Bind::Text(row.commit_id.clone()),
                Bind::Int(row.seq as i64),
                Bind::Int(row.v as i64),
            ],
            Some("identity_key"),
        )
        .await?;
    }
    for row in &batch.states {
        insert_row(
            &mut tx,
            dialect,
            "g_state",
            "scope, key, value, commit_id, seq, v",
            vec![
                Bind::Text(row.scope.clone()),
                Bind::Text(row.key.clone()),
                Bind::Text(serde_json::to_string(&row.value)?),
                Bind::Text(row.commit_id.clone()),
                Bind::Int(row.seq as i64),
                Bind::Int(row.v as i64),
            ],
        )
        .await?;
    }
    for row in &batch.vectors {
        write_vector(&mut tx, dialect, row).await?;
    }
    for row in &batch.texts {
        write_text(&mut tx, dialect, row).await?;
    }

    tx.commit().await?;
    Ok(versions)
}

async fn write_vector(tx: &mut Tx<'_>, dialect: Dialect, row: &VectorRow) -> Result<()> {
    let variant = row.variant.clone().unwrap_or_default();
    match dialect {
        Dialect::Sqlite => insert_row(
            tx,
            dialect,
            "g_vectors",
            "rid, kind, variant, model, dims, vec, v",
            vec![
                Bind::Text(row.rid.clone()),
                Bind::Text(row.kind.clone()),
                Bind::Text(variant),
                Bind::Text(row.model.clone()),
                Bind::Int(row.vector.len() as i64),
                Bind::Bytes(vec_bytes(&row.vector)),
                Bind::Int(row.v as i64),
            ],
        )
        .await,
        Dialect::Postgres => {
            let table = format!("g_vec_{}", row.vector.len());
            let mut ddl = Sql::new(dialect);
            ddl.push("CREATE TABLE IF NOT EXISTS ").push(&table);
            ddl.push(" (rid TEXT NOT NULL, kind TEXT NOT NULL, variant TEXT NOT NULL DEFAULT '', model TEXT NOT NULL DEFAULT '', embedding vector(");
            ddl.int(row.vector.len() as i64);
            ddl.push(") NOT NULL, v BIGINT NOT NULL, PRIMARY KEY(rid, kind, variant, model, v))");
            tx.exec(&ddl).await?;

            let mut sql = Sql::new(dialect);
            sql.push("INSERT INTO ").push(&table);
            sql.push(" (rid, kind, variant, model, embedding, v) VALUES (");
            sql.text(row.rid.clone());
            sql.push(", ");
            sql.text(row.kind.clone());
            sql.push(", ");
            sql.text(variant);
            sql.push(", ");
            sql.text(row.model.clone());
            sql.push(", ");
            sql.text(vec_text(&row.vector));
            sql.push("::vector, ");
            sql.u64(row.v);
            sql.push(")");
            tx.exec(&sql).await?;
            Ok(())
        }
    }
}

async fn write_text(tx: &mut Tx<'_>, dialect: Dialect, row: &TextRow) -> Result<()> {
    let variant = row.variant.clone().unwrap_or_default();
    let table = match dialect {
        Dialect::Sqlite => "g_fts",
        Dialect::Postgres => "g_docs",
    };
    insert_row(
        tx,
        dialect,
        table,
        "rid, kind, variant, body, v",
        vec![
            Bind::Text(row.rid.clone()),
            Bind::Text(row.kind.clone()),
            Bind::Text(variant),
            Bind::Text(row.body.clone()),
            Bind::Int(row.v as i64),
        ],
    )
    .await
}

/// 当前各表版本。
pub async fn versions(db: &Db) -> Result<BTreeMap<String, u64>> {
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT name, version FROM g_versions ORDER BY name");
    let rows = db.fetch(&sql).await?;
    let mut versions = BTreeMap::new();
    for row in rows {
        versions.insert(row.text("name")?, row.u64("version")?);
    }
    Ok(versions)
}

/// 最早可见版本；空表按 0 处理。
pub async fn earliest(db: &Db, logical: &str) -> Result<u64> {
    let table = physical(logical);
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT CAST(COALESCE(MIN(v), 0) AS BIGINT) AS m FROM ")
        .push(&table);
    Ok(db.fetch_int(&sql).await?.unwrap_or(0).max(0) as u64)
}

/// 回退到指定版本：删掉更晚写入的行并同步计数器。用于恢复半批刷写。
pub async fn restore(db: &Db, targets: &BTreeMap<String, u64>) -> Result<()> {
    let dialect = db.dialect();
    let mut tx = db.begin().await?;
    for (logical, target) in targets {
        if !LOGICAL_TABLES.contains(&logical.as_str()) {
            continue;
        }
        let table = physical(logical);
        let mut delete = Sql::new(dialect);
        delete.push("DELETE FROM ").push(&table).push(" WHERE v > ");
        delete.u64(*target);
        tx.exec(&delete).await?;
        let mut set = Sql::new(dialect);
        set.push("UPDATE g_versions SET version = ");
        set.u64(*target);
        set.push(" WHERE name = ");
        set.text(logical.clone());
        tx.exec(&set).await?;
    }
    tx.commit().await
}

/// 读出某个版本组合下的全部行。
pub async fn read_at(db: &Db, versions: &BTreeMap<String, u64>) -> Result<Dataset> {
    Ok(Dataset {
        snapshot: None,
        resources: read_resources(db, version_of(versions, "resources")).await?,
        marks: read_marks(db, version_of(versions, "resource_marks")).await?,
        facets: read_facets(db, version_of(versions, "facets")).await?,
        relations: read_relations(db, version_of(versions, "relations")).await?,
        identities: read_identities(db, version_of(versions, "identities")).await?,
        states: read_states(db, version_of(versions, "state")).await?,
    })
}

async fn read_resources(db: &Db, version: u64) -> Result<Vec<ResourceRow>> {
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT rid, ns, source_id, created_at, seq, v FROM g_resources WHERE v <= ");
    sql.u64(version);
    sql.push(" ORDER BY seq, rid");
    let rows = db.fetch(&sql).await?;
    rows.iter()
        .map(|row| {
            Ok(ResourceRow {
                rid: row.text("rid")?,
                ns: row.text("ns")?,
                source_id: row.opt_text("source_id")?,
                created_at: row.text("created_at")?,
                seq: row.u64("seq")?,
                v: row.u64("v")?,
            })
        })
        .collect()
}

async fn read_marks(db: &Db, version: u64) -> Result<Vec<MarkRow>> {
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT rid, labels, tombstone, reason, seq, v FROM g_resource_marks WHERE v <= ");
    sql.u64(version);
    sql.push(" ORDER BY seq");
    let rows = db.fetch(&sql).await?;
    rows.iter()
        .map(|row| {
            Ok(MarkRow {
                rid: row.text("rid")?,
                labels: serde_json::from_str(&row.text("labels")?)?,
                tombstone: row.bool("tombstone")?,
                reason: row.opt_text("reason")?,
                seq: row.u64("seq")?,
                v: row.u64("v")?,
            })
        })
        .collect()
}

async fn read_facets(db: &Db, version: u64) -> Result<Vec<FacetRow>> {
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT rid, kind, variant, version, content_hash, media_type, size, body_text, body_struct, blob_ref, inputs, params_hash, producer_module, producer_version, config_generation, commit_id, created_at, seq, v FROM g_facets WHERE v <= ");
    sql.u64(version);
    sql.push(" ORDER BY seq, rid");
    let rows = db.fetch(&sql).await?;
    rows.iter().map(facet_of).collect()
}

fn facet_of(row: &DbRow) -> Result<FacetRow> {
    let variant = row.text("variant")?;
    Ok(FacetRow {
        rid: row.text("rid")?,
        kind: row.text("kind")?,
        variant: if variant.is_empty() { None } else { Some(variant) },
        version: row.u64("version")?,
        content_hash: row.text("content_hash")?,
        media_type: row.opt_text("media_type")?,
        size: row.u64("size")?,
        body_text: row.opt_text("body_text")?,
        body_struct: row.opt_text("body_struct")?,
        blob_ref: row.opt_text("blob_ref")?,
        inputs: serde_json::from_str(&row.text("inputs")?)?,
        params_hash: row.text("params_hash")?,
        producer_module: row.text("producer_module")?,
        producer_version: row.text("producer_version")?,
        config_generation: row.u64("config_generation")?,
        commit_id: row.text("commit_id")?,
        created_at: row.text("created_at")?,
        seq: row.u64("seq")?,
        v: row.u64("v")?,
    })
}

async fn read_relations(db: &Db, version: u64) -> Result<Vec<RelationRow>> {
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT src_rid, rel, dst_rid, attrs, commit_id, seq, v FROM g_relations WHERE v <= ");
    sql.u64(version);
    sql.push(" ORDER BY seq");
    let rows = db.fetch(&sql).await?;
    rows.iter()
        .map(|row| {
            let attrs = row.opt_text("attrs")?;
            Ok(RelationRow {
                src_rid: row.text("src_rid")?,
                rel: row.text("rel")?,
                dst_rid: row.text("dst_rid")?,
                attrs: match attrs {
                    Some(text) => Some(serde_json::from_str(&text)?),
                    None => None,
                },
                commit_id: row.text("commit_id")?,
                seq: row.u64("seq")?,
                v: row.u64("v")?,
            })
        })
        .collect()
}

async fn read_identities(db: &Db, version: u64) -> Result<Vec<IdentityRow>> {
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT identity_key, rid, scheme, commit_id, seq, v FROM g_identities WHERE v <= ");
    sql.u64(version);
    sql.push(" ORDER BY seq");
    let rows = db.fetch(&sql).await?;
    rows.iter()
        .map(|row| {
            Ok(IdentityRow {
                identity_key: row.text("identity_key")?,
                rid: row.text("rid")?,
                scheme: row.text("scheme")?,
                commit_id: row.text("commit_id")?,
                seq: row.u64("seq")?,
                v: row.u64("v")?,
            })
        })
        .collect()
}

async fn read_states(db: &Db, version: u64) -> Result<Vec<StateRow>> {
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT scope, key, value, commit_id, seq, v FROM g_state WHERE v <= ");
    sql.u64(version);
    sql.push(" ORDER BY seq");
    let rows = db.fetch(&sql).await?;
    rows.iter()
        .map(|row| {
            Ok(StateRow {
                scope: row.text("scope")?,
                key: row.text("key")?,
                value: serde_json::from_str(&row.text("value")?)?,
                commit_id: row.text("commit_id")?,
                seq: row.u64("seq")?,
                v: row.u64("v")?,
            })
        })
        .collect()
}

/// 全部快照，按序号升序。
pub async fn snapshots(db: &Db) -> Result<Vec<SnapshotRow>> {
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT snapshot_seq, table_versions, commit_ids, kind, config_generation, published_at, v FROM g_snapshots ORDER BY snapshot_seq");
    let rows = db.fetch(&sql).await?;
    rows.iter()
        .map(|row| {
            Ok(SnapshotRow {
                seq: row.u64("snapshot_seq")?,
                versions: serde_json::from_str(&row.text("table_versions")?)?,
                commit_ids: serde_json::from_str(&row.text("commit_ids")?)?,
                kind: row.text("kind")?,
                config_generation: row.u64("config_generation")?,
                published_at: row.text("published_at")?,
                v: row.u64("v")?,
            })
        })
        .collect()
}

/// 最近一个快照。
pub async fn latest_snapshot(db: &Db) -> Result<Option<SnapshotRow>> {
    Ok(snapshots(db).await?.into_iter().max_by_key(|row| row.seq))
}

/// 发布快照。版本取当前各表版本。
pub async fn write_snapshot(
    db: &Db,
    commit_ids: &[String],
    kind: &str,
    config_generation: u64,
    published_at: &str,
) -> Result<SnapshotRow> {
    let dialect = db.dialect();
    let mut versions = versions(db).await?;
    let mut tx = db.begin().await?;
    let v = bump_version(&mut tx, dialect, "snapshots").await?;
    versions.insert("snapshots".into(), v);
    let seq = alloc_counter_tx(&mut tx, "snapshot", 1).await?;
    let row = SnapshotRow {
        seq,
        versions,
        commit_ids: commit_ids.to_vec(),
        kind: kind.to_string(),
        config_generation,
        published_at: published_at.to_string(),
        v,
    };
    insert_row(
        &mut tx,
        dialect,
        "g_snapshots",
        "snapshot_seq, table_versions, commit_ids, kind, config_generation, published_at, v",
        vec![
            Bind::Int(row.seq as i64),
            Bind::Text(serde_json::to_string(&row.versions)?),
            Bind::Text(serde_json::to_string(&row.commit_ids)?),
            Bind::Text(row.kind.clone()),
            Bind::Int(row.config_generation as i64),
            Bind::Text(row.published_at.clone()),
            Bind::Int(row.v as i64),
        ],
    )
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// 某个快照版本下按哈希取 blob 行。
pub async fn blob_row(
    db: &Db,
    versions: &BTreeMap<String, u64>,
    sha: &str,
) -> Result<BlobRow> {
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT sha256, size, media_type, tier, inline, created_at, seq, v FROM g_blobs WHERE sha256 = ");
    sql.text(sha.to_string());
    sql.push(" AND v <= ");
    sql.u64(version_of(versions, "blobs"));
    let row = db
        .fetch_one(&sql)
        .await?
        .ok_or_else(|| Error::MissingBlob(sha.to_string()))?;
    Ok(BlobRow {
        sha256: row.text("sha256")?,
        size: row.u64("size")?,
        media_type: row.opt_text("media_type")?,
        tier: BlobTier::parse(&row.text("tier")?)?,
        inline: row.opt_bytes("inline")?,
        created_at: row.text("created_at")?,
        seq: row.u64("seq")?,
        v: row.u64("v")?,
    })
}

/// 测试用行计数。
pub async fn count(db: &Db, logical: &str) -> Result<u64> {
    let table = physical(logical);
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT CAST(COUNT(*) AS BIGINT) AS n FROM ")
        .push(&table);
    Ok(db.fetch_int(&sql).await?.unwrap_or(0).max(0) as u64)
}

/// 单个资源的最新标记，按 seq 取最大。
pub async fn latest_mark(db: &Db, versions: &BTreeMap<String, u64>, rid: &str) -> Result<Option<MarkRow>> {
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT rid, labels, tombstone, reason, seq, v FROM g_resource_marks WHERE rid = ");
    sql.text(rid.to_string());
    sql.push(" AND v <= ");
    sql.u64(version_of(versions, "resource_marks"));
    sql.push(" ORDER BY seq DESC LIMIT 1");
    let row = db.fetch_one(&sql).await?;
    row.map(|row| {
        Ok(MarkRow {
            rid: row.text("rid")?,
            labels: serde_json::from_str(&row.text("labels")?)?,
            tombstone: row.bool("tombstone")?,
            reason: row.opt_text("reason")?,
            seq: row.u64("seq")?,
            v: row.u64("v")?,
        })
    })
    .transpose()
}
