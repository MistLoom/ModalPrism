//! 向量与全文索引。写点在 `data::write` 里，和行数据同事务；这里只做检索。
//!
//! SQLite：向量默认用规范表做精确扫描；装载了 `sqlite-vec` 时改走 `vec_distance_cosine`。
//! 全文固定走 FTS5（`unicode61` 或 `trigram`）。
//! PostgreSQL：向量走 pgvector 的 `<=>`，全文走 `tsvector`。

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::config::Tokenizer;
use crate::db::Db;
use crate::error::{Error, Result};
use crate::sql::{Dialect, Sql};

/// 一路检索候选。
#[derive(Clone, Debug, PartialEq)]
pub struct Score {
    pub rid: String,
    pub score: f32,
}

pub struct VectorQuery<'a> {
    pub vector: &'a [f32],
    pub k: usize,
    pub kind: Option<&'a str>,
    pub variant: Option<&'a str>,
    pub model: Option<&'a str>,
    /// 快照里的 `vectors` 版本。
    pub version: u64,
}

pub struct TextQuery<'a> {
    pub text: &'a str,
    pub k: usize,
    pub kind: Option<&'a str>,
    pub variant: Option<&'a str>,
    /// 快照里的 `text` 版本。
    pub version: u64,
}

pub struct IndexStore {
    db: Arc<Db>,
    tokenizer: Tokenizer,
    sqlite_vec: bool,
}

impl IndexStore {
    pub async fn open(db: Arc<Db>, tokenizer: Tokenizer) -> Result<Self> {
        let sqlite_vec = match db.dialect() {
            Dialect::Sqlite => probe_vec(&db).await,
            Dialect::Postgres => false,
        };
        Ok(Self {
            db,
            tokenizer,
            sqlite_vec,
        })
    }

    /// 是否检测到 sqlite-vec。
    pub fn sqlite_vec(&self) -> bool {
        self.sqlite_vec
    }

    pub async fn search_vectors(&self, query: &VectorQuery<'_>) -> Result<Vec<Score>> {
        if query.vector.is_empty() {
            return Err(Error::VectorDims {
                expected: 1,
                actual: 0,
            });
        }
        if query.k == 0 {
            return Ok(Vec::new());
        }
        match self.db.dialect() {
            Dialect::Sqlite if self.sqlite_vec => self.sqlite_vec_search(query).await,
            Dialect::Sqlite => self.sqlite_scan(query).await,
            Dialect::Postgres => self.postgres_search(query).await,
        }
    }

    pub async fn search_text(&self, query: &TextQuery<'_>) -> Result<Vec<Score>> {
        if query.k == 0 {
            return Ok(Vec::new());
        }
        match self.db.dialect() {
            Dialect::Sqlite => self.sqlite_text(query).await,
            Dialect::Postgres => self.postgres_text(query).await,
        }
    }

    /// 精确扫描：读回候选向量，去重后在内存里算余弦。
    async fn sqlite_scan(&self, query: &VectorQuery<'_>) -> Result<Vec<Score>> {
        let mut sql = Sql::new(Dialect::Sqlite);
        sql.push("SELECT rid, variant, model, vec, v FROM g_vectors WHERE v <= ");
        sql.u64(query.version);
        filter_columns(&mut sql, query.kind, query.variant, query.model);
        sql.push(" ORDER BY rid, variant, model, v DESC");
        let rows = self.db.fetch(&sql).await?;
        let mut heads: BTreeMap<(String, String, String), (u64, Vec<f32>)> = BTreeMap::new();
        for row in rows {
            let key = (row.text("rid")?, row.text("variant")?, row.text("model")?);
            let version = row.u64("v")?;
            if heads.get(&key).is_some_and(|(seen, _)| *seen >= version) {
                continue;
            }
            let vector = decode_vector(&row.bytes("vec")?)?;
            heads.insert(key, (version, vector));
        }
        let mut scores: Vec<Score> = heads
            .into_iter()
            .map(|((rid, _, _), (_, vector))| Score {
                rid,
                score: cosine(query.vector, &vector),
            })
            .collect();
        sort_scores(&mut scores, query.k);
        Ok(scores)
    }

    /// sqlite-vec：交给 `vec_distance_cosine`，用窗口函数取每个资源的最新向量。
    async fn sqlite_vec_search(&self, query: &VectorQuery<'_>) -> Result<Vec<Score>> {
        let mut sql = Sql::new(Dialect::Sqlite);
        sql.push("SELECT rid, distance FROM (SELECT gv.rid AS rid, vec_distance_cosine(gv.vec, ");
        sql.bytes(vec_bytes(query.vector));
        sql.push(") AS distance, ROW_NUMBER() OVER (PARTITION BY gv.rid, gv.variant, gv.model ORDER BY gv.v DESC) AS rn FROM g_vectors gv WHERE gv.v <= ");
        sql.u64(query.version);
        filter_columns(&mut sql, query.kind, query.variant, query.model);
        sql.push(") t WHERE rn = 1 ORDER BY distance LIMIT ");
        sql.u64(query.k as u64);
        let rows = self.db.fetch(&sql).await?;
        rows.iter()
            .map(|row| {
                Ok(Score {
                    rid: row.text("rid")?,
                    score: 1.0 - row.real("distance")? as f32,
                })
            })
            .collect()
    }

    async fn postgres_search(&self, query: &VectorQuery<'_>) -> Result<Vec<Score>> {
        let table = format!("g_vec_{}", query.vector.len());
        let mut sql = Sql::new(Dialect::Postgres);
        sql.push("SELECT rid, distance FROM (SELECT DISTINCT ON (rid) rid, embedding <=> ");
        sql.text(vec_text(query.vector));
        sql.push("::vector AS distance FROM ")
            .push(&table)
            .push(" WHERE v <= ");
        sql.u64(query.version);
        filter_columns(&mut sql, query.kind, query.variant, query.model);
        sql.push(" ORDER BY rid, v DESC) t ORDER BY distance LIMIT ");
        sql.u64(query.k as u64);
        let rows = self.db.fetch(&sql).await?;
        rows.iter()
            .map(|row| {
                Ok(Score {
                    rid: row.text("rid")?,
                    score: 1.0 - row.real("distance")? as f32,
                })
            })
            .collect()
    }

    async fn sqlite_text(&self, query: &TextQuery<'_>) -> Result<Vec<Score>> {
        let expression = match self.tokenizer {
            Tokenizer::Unicode => fts_expression(query.text),
            Tokenizer::Trigram => fts_trigram_expression(query.text),
        };
        if expression.is_empty() {
            return Ok(Vec::new());
        }
        let mut sql = Sql::new(Dialect::Sqlite);
        // FTS5 的 bm25 不能出现在子查询里，否则报
        // "unable to use function bm25 in the requested context"。用 heads
        // 子查询挑出每个 (rid, kind, variant) 在快照内的最新版本，bm25 留在最外层。
        sql.push("SELECT g_fts.rid AS rid, bm25(g_fts) AS rank FROM g_fts JOIN (SELECT rid AS hrid, kind AS hkind, variant AS hvariant, MAX(v) AS hv FROM g_fts WHERE v <= ");
        sql.u64(query.version);
        sql.push(" GROUP BY rid, kind, variant) h ON h.hrid = g_fts.rid AND h.hkind = g_fts.kind AND h.hvariant = g_fts.variant AND h.hv = g_fts.v WHERE g_fts MATCH ");
        sql.text(expression);
        if let Some(kind) = query.kind {
            sql.push(" AND g_fts.kind = ");
            sql.text(kind.to_string());
        }
        if let Some(variant) = query.variant {
            sql.push(" AND g_fts.variant = ");
            sql.text(variant.to_string());
        }
        sql.push(" ORDER BY rank LIMIT ");
        sql.u64(query.k.saturating_mul(4).max(1) as u64);
        let rows = self.db.fetch(&sql).await?;
        let mut heads: BTreeMap<String, f32> = BTreeMap::new();
        for row in &rows {
            let rid = row.text("rid")?;
            let rank = row.real("rank")? as f32;
            heads
                .entry(rid)
                .and_modify(|seen| *seen = seen.min(rank))
                .or_insert(rank);
        }
        let mut scores: Vec<Score> = heads
            .into_iter()
            .map(|(rid, rank)| Score { rid, score: -rank })
            .collect();
        sort_scores(&mut scores, query.k);
        Ok(scores)
    }

    async fn postgres_text(&self, query: &TextQuery<'_>) -> Result<Vec<Score>> {
        if query.text.trim().is_empty() {
            return Ok(Vec::new());
        }
        let mut sql = Sql::new(Dialect::Postgres);
        // 每个 (rid, kind, variant) 只取快照内的最新版本，再按 rid 取最高分。
        sql.push("SELECT rid, MAX(score) AS score FROM (SELECT DISTINCT ON (rid, kind, variant) rid, kind, variant, ts_rank(to_tsvector('simple', body), plainto_tsquery('simple', ");
        sql.text(query.text.to_string());
        sql.push(")) AS score FROM g_docs WHERE to_tsvector('simple', body) @@ plainto_tsquery('simple', ");
        sql.text(query.text.to_string());
        sql.push(") AND v <= ");
        sql.u64(query.version);
        if let Some(kind) = query.kind {
            sql.push(" AND kind = ");
            sql.text(kind.to_string());
        }
        if let Some(variant) = query.variant {
            sql.push(" AND variant = ");
            sql.text(variant.to_string());
        }
        sql.push(" ORDER BY rid, kind, variant, v DESC) t GROUP BY rid ORDER BY score DESC LIMIT ");
        sql.u64(query.k as u64);
        let rows = self.db.fetch(&sql).await?;
        rows.iter()
            .map(|row| {
                Ok(Score {
                    rid: row.text("rid")?,
                    score: row.real("score")? as f32,
                })
            })
            .collect()
    }
}

async fn probe_vec(db: &Db) -> bool {
    let sql = Sql::raw(Dialect::Sqlite, "SELECT vec_version() AS v");
    matches!(db.fetch(&sql).await, Ok(rows) if !rows.is_empty())
}

fn filter_columns(sql: &mut Sql, kind: Option<&str>, variant: Option<&str>, model: Option<&str>) {
    if let Some(kind) = kind {
        sql.push(" AND kind = ");
        sql.text(kind.to_string());
    }
    if let Some(variant) = variant {
        sql.push(" AND variant = ");
        sql.text(variant.to_string());
    }
    if let Some(model) = model {
        sql.push(" AND model = ");
        sql.text(model.to_string());
    }
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

fn decode_vector(bytes: &[u8]) -> Result<Vec<f32>> {
    if bytes.len() % 4 != 0 {
        return Err(Error::ColumnType("向量字节长度不是 4 的倍数".into()));
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}

fn cosine(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut left_norm = 0.0f32;
    let mut right_norm = 0.0f32;
    for (a, b) in left.iter().zip(right.iter()) {
        dot += a * b;
        left_norm += a * a;
        right_norm += b * b;
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        return 0.0;
    }
    dot / (left_norm.sqrt() * right_norm.sqrt())
}

fn sort_scores(scores: &mut Vec<Score>, k: usize) {
    scores.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.rid.cmp(&right.rid))
    });
    scores.truncate(k);
}

/// FTS5 查询表达式：每个词加引号，避免用户输入被当成语法。
fn fts_expression(text: &str) -> String {
    text.split_whitespace()
        .filter(|token| !token.is_empty())
        .map(|token| format!("\"{}\"", token.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// trigram 分词器要求词长至少 3。
fn fts_trigram_expression(text: &str) -> String {
    let tokens: Vec<String> = text
        .split_whitespace()
        .filter(|token| token.chars().count() >= 3)
        .map(|token| format!("\"{}\"", token.replace('"', "")))
        .collect();
    tokens.join(" ")
}
