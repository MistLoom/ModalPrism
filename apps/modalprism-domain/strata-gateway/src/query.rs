//! 统一查询入口：过滤下推到 SQL，向量与全文各出候选，按倒数排名融合。
//!
//! 业务只提交 `Request`，方言差异留在各后端实现内部。

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};
use strata_model::{Expr, FacetSelector, OrderBy, QueryFrom, Search};

use crate::config::FusionOptions;
use crate::db::Db;
use crate::error::{Error, Result};
use crate::index::{IndexStore, Score, TextQuery, VectorQuery};
use crate::sql::Sql;

/// 查询请求。`search` 只用于 `from = Resources`。
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub from: QueryFrom,
    pub filter: Option<Expr>,
    pub has_facets: Vec<FacetSelector>,
    pub search: Option<Search>,
    /// 快照序号；缺省读最新。
    pub as_of: Option<u64>,
    pub order_by: Vec<OrderBy>,
    pub limit: Option<u32>,
    /// 上一页最后一个 `seq`。
    pub cursor: Option<u64>,
    /// 是否返回已写入墓碑的资源；缺省不返回。
    pub include_tombstoned: bool,
}

impl Request {
    pub fn new(from: QueryFrom) -> Self {
        Self {
            from,
            filter: None,
            has_facets: Vec::new(),
            search: None,
            as_of: None,
            order_by: Vec::new(),
            limit: None,
            cursor: None,
            include_tombstoned: false,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.limit == Some(0) {
            return Err(Error::InvalidFilter("limit 不能为 0".into()));
        }
        if self.search.is_some() && self.from != QueryFrom::Resources {
            return Err(Error::Unsupported(
                "向量/全文检索只支持 resources 起点".into(),
            ));
        }
        if self.search.is_some() && !self.has_facets.is_empty() {
            return Err(Error::Unsupported("检索与 hasFacets 不能同时使用".into()));
        }
        if self.cursor.is_some() && !self.order_by.is_empty() {
            return Err(Error::Unsupported("游标与显式排序不能同时使用".into()));
        }
        if self.search.as_ref().is_some_and(|search| search.k == 0) {
            return Err(Error::InvalidFilter("检索 k 不能为 0".into()));
        }
        if self.order_by.iter().any(|order| order.field.is_empty()) {
            return Err(Error::InvalidFilter("排序字段为空".into()));
        }
        Ok(())
    }
}

/// 命中来自哪一路。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Filter,
    Vector,
    Text,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub rid: String,
    pub score: f32,
    pub channels: Vec<Channel>,
    pub row: Value,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Response {
    pub rows: Vec<Hit>,
    pub next_cursor: Option<u64>,
    pub snapshot: Option<u64>,
}

/// 一基础行的投影，带用于游标与排序的 `seq`。
struct Base {
    rid: String,
    seq: u64,
    row: Value,
}

pub async fn execute(
    db: &Db,
    index: &IndexStore,
    fusion: &FusionOptions,
    versions: &BTreeMap<String, u64>,
    snapshot: Option<u64>,
    request: &Request,
) -> Result<Response> {
    request.validate()?;
    let limit = request.limit.unwrap_or(50).max(1) as usize;
    let candidates = if request.search.is_some() {
        limit.saturating_mul(4).max(64)
    } else {
        limit
    };

    let mut bases = base_rows(db, versions, request, candidates).await?;
    let next_cursor = if request.search.is_none() && bases.len() > limit {
        Some(bases[limit - 1].seq)
    } else {
        None
    };
    if request.search.is_none() {
        bases.truncate(limit);
        return Ok(Response {
            rows: bases
                .into_iter()
                .map(|base| Hit {
                    rid: base.rid,
                    score: 0.0,
                    channels: vec![Channel::Filter],
                    row: base.row,
                })
                .collect(),
            next_cursor,
            snapshot,
        });
    }

    let Search {
        text,
        vector,
        variant,
        k,
    } = request
        .search
        .clone()
        .ok_or_else(|| Error::InvalidFilter("检索请求缺失".into()))?;
    let k = k as usize;

    let mut channels: Vec<(Channel, f32, Vec<Score>)> = Vec::new();
    if request.filter.is_some() {
        channels.push((
            Channel::Filter,
            fusion.filter_weight,
            bases
                .iter()
                .take(k)
                .enumerate()
                .map(|(rank, base)| Score {
                    rid: base.rid.clone(),
                    score: -(rank as f32),
                })
                .collect(),
        ));
    }
    if let Some(vector) = &vector {
        if !vector.is_empty() {
            let scores = index
                .search_vectors(&VectorQuery {
                    vector,
                    k,
                    kind: Some("embedding"),
                    variant: variant.as_deref(),
                    model: None,
                    version: versions.get("vectors").copied().unwrap_or(0),
                })
                .await?;
            channels.push((Channel::Vector, fusion.vector_weight, scores));
        }
    }
    if let Some(text) = &text {
        if !text.trim().is_empty() {
            let scores = index
                .search_text(&TextQuery {
                    text,
                    k,
                    kind: None,
                    variant: variant.as_deref(),
                    version: versions.get("text").copied().unwrap_or(0),
                })
                .await?;
            channels.push((Channel::Text, fusion.text_weight, scores));
        }
    }
    if channels.is_empty() {
        return Ok(Response {
            rows: Vec::new(),
            next_cursor: None,
            snapshot,
        });
    }
    if channels.len() == 1 && channels[0].0 == Channel::Filter {
        // 只有过滤一路：按基础顺序返回，避免无谓的融合。
        bases.truncate(limit);
        return Ok(Response {
            rows: bases
                .into_iter()
                .map(|base| Hit {
                    rid: base.rid,
                    score: 0.0,
                    channels: vec![Channel::Filter],
                    row: base.row,
                })
                .collect(),
            next_cursor,
            snapshot,
        });
    }

    let mut fused: BTreeMap<String, (f32, Vec<Channel>)> = BTreeMap::new();
    for (channel, weight, scores) in &channels {
        if *weight == 0.0 {
            continue;
        }
        for (rank, score) in scores.iter().enumerate() {
            let slot = fused.entry(score.rid.clone()).or_insert((0.0, Vec::new()));
            slot.0 += weight / (fusion.rrf_k as f32 + rank as f32 + 1.0);
            if !slot.1.contains(channel) {
                slot.1.push(*channel);
            }
        }
    }

    let mut ranked: Vec<(String, f32, Vec<Channel>)> = fused
        .into_iter()
        .map(|(rid, (score, channels))| (rid, score, channels))
        .collect();
    ranked.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    // 检索直连索引，墓碑过滤要在这里补一次，才能与基础行路径一致。
    if request.from == QueryFrom::Resources && !request.include_tombstoned {
        let rids: Vec<String> = ranked.iter().map(|(rid, _, _)| rid.clone()).collect();
        let hidden = tombstoned_rids(db, versions, &rids).await?;
        ranked.retain(|(rid, _, _)| !hidden.contains(rid));
    }
    ranked.truncate(limit);

    let mut payloads: BTreeMap<String, Value> =
        bases.into_iter().map(|base| (base.rid, base.row)).collect();
    let missing: Vec<String> = ranked
        .iter()
        .map(|(rid, _, _)| rid.clone())
        .filter(|rid| !payloads.contains_key(rid))
        .collect();
    if !missing.is_empty() {
        for base in resource_rows_by_rid(db, versions, &missing).await? {
            payloads.insert(base.rid, base.row);
        }
    }

    let rows = ranked
        .into_iter()
        .map(|(rid, score, channels)| Hit {
            row: payloads.remove(&rid).unwrap_or_else(|| {
                Value::Object(Map::from_iter([("rid".into(), Value::String(rid.clone()))]))
            }),
            rid,
            score,
            channels,
        })
        .collect();
    Ok(Response {
        rows,
        next_cursor: None,
        snapshot,
    })
}

fn table_of(from: QueryFrom) -> &'static str {
    match from {
        QueryFrom::Resources => "g_resources",
        QueryFrom::Facets => "g_facets",
        QueryFrom::Relations => "g_relations",
    }
}

fn version_key(from: QueryFrom) -> &'static str {
    match from {
        QueryFrom::Resources => "resources",
        QueryFrom::Facets => "facets",
        QueryFrom::Relations => "relations",
    }
}

fn column_of(from: QueryFrom, field: &str) -> Option<&'static str> {
    let column = match (from, field) {
        (QueryFrom::Resources, "rid") => "rid",
        (QueryFrom::Resources, "ns") => "ns",
        (QueryFrom::Resources, "sourceId") => "source_id",
        (QueryFrom::Resources, "createdAt") => "created_at",
        (QueryFrom::Resources, "seq") => "seq",
        (QueryFrom::Facets, "rid") => "rid",
        (QueryFrom::Facets, "kind") => "kind",
        (QueryFrom::Facets, "variant") => "variant",
        (QueryFrom::Facets, "version") => "version",
        (QueryFrom::Facets, "contentHash") => "content_hash",
        (QueryFrom::Facets, "mediaType") => "media_type",
        (QueryFrom::Facets, "size") => "size",
        (QueryFrom::Facets, "paramsHash") => "params_hash",
        (QueryFrom::Facets, "producerModule") => "producer_module",
        (QueryFrom::Facets, "producerVersion") => "producer_version",
        (QueryFrom::Facets, "configGeneration") => "config_generation",
        (QueryFrom::Facets, "commitId") => "commit_id",
        (QueryFrom::Facets, "seq") => "seq",
        (QueryFrom::Relations, "srcRid") => "src_rid",
        (QueryFrom::Relations, "rel") => "rel",
        (QueryFrom::Relations, "dstRid") => "dst_rid",
        (QueryFrom::Relations, "seq") => "seq",
        _ => return None,
    };
    Some(column)
}

fn bind_value(sql: &mut Sql, value: &Value) -> Result<()> {
    match value {
        Value::String(text) => {
            sql.text(text.clone());
            Ok(())
        }
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                sql.int(value);
                Ok(())
            } else if let Some(value) = number.as_u64() {
                sql.u64(value);
                Ok(())
            } else {
                Err(Error::InvalidFilter(format!("不支持的数字: {number}")))
            }
        }
        Value::Bool(value) => {
            sql.int(if *value { 1 } else { 0 });
            Ok(())
        }
        Value::Null => Err(Error::InvalidFilter("不支持直接过滤 null".into())),
        other => Err(Error::InvalidFilter(format!("不支持的过滤值: {other}"))),
    }
}

fn pair(value: &Value) -> Result<(&str, &Value)> {
    let items = value
        .as_array()
        .ok_or_else(|| Error::InvalidFilter("操作数必须是数组".into()))?;
    if items.len() != 2 {
        return Err(Error::InvalidFilter("操作数必须是 [字段, 值]".into()));
    }
    let field = items[0]
        .as_str()
        .ok_or_else(|| Error::InvalidFilter("字段名必须是字符串".into()))?;
    Ok((field, &items[1]))
}

fn compile(
    sql: &mut Sql,
    from: QueryFrom,
    versions: &BTreeMap<String, u64>,
    expr: &Expr,
) -> Result<()> {
    let map = expr.as_map();
    if map.len() != 1 {
        return Err(Error::InvalidFilter("过滤表达式必须只有一个顶层键".into()));
    }
    let (key, value) = map
        .iter()
        .next()
        .ok_or_else(|| Error::InvalidFilter("过滤表达式为空".into()))?;
    match key.as_str() {
        "all" => {
            let items = value
                .as_array()
                .ok_or_else(|| Error::InvalidFilter("all 需要数组".into()))?;
            if items.is_empty() {
                return Err(Error::InvalidFilter("all 不能为空".into()));
            }
            sql.push("(");
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    sql.push(" AND ");
                }
                let nested = Expr::try_from(item.clone())
                    .map_err(|error| Error::InvalidFilter(error.to_string()))?;
                compile(sql, from, versions, &nested)?;
            }
            sql.push(")");
            Ok(())
        }
        "eq" | "neq" | "gte" | "gt" | "lte" | "lt" => {
            let (field, expected) = pair(value)?;
            let column = column_of(from, field)
                .ok_or_else(|| Error::UnknownField(format!("{from:?}.{field}")))?;
            let op = match key.as_str() {
                "eq" => "=",
                "neq" => "<>",
                "gte" => ">=",
                "gt" => ">",
                "lte" => "<=",
                _ => "<",
            };
            sql.push(column).push(" ").push(op).push(" ");
            bind_value(sql, expected)?;
            Ok(())
        }
        "in" => {
            let (field, expected) = pair(value)?;
            let column = column_of(from, field)
                .ok_or_else(|| Error::UnknownField(format!("{from:?}.{field}")))?;
            let items = expected
                .as_array()
                .ok_or_else(|| Error::InvalidFilter("in 需要数组".into()))?;
            if items.is_empty() {
                return Err(Error::InvalidFilter("in 不能为空".into()));
            }
            sql.push(column).push(" IN (");
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    sql.push(", ");
                }
                bind_value(sql, item)?;
            }
            sql.push(")");
            Ok(())
        }
        "label" => {
            if from != QueryFrom::Resources {
                return Err(Error::Unsupported("label 过滤只支持 resources".into()));
            }
            let (name, expected) = pair(value)?;
            let expected = expected
                .as_str()
                .ok_or_else(|| Error::InvalidFilter("标签值必须是字符串".into()))?;
            sql.push("EXISTS (SELECT 1 FROM g_resource_marks m WHERE m.rid = g_resources.rid AND m.v <= ");
            sql.u64(versions.get("resource_marks").copied().unwrap_or(0));
            sql.push(" AND m.labels LIKE ");
            sql.text(format!("%\"{}={}\"%", name, expected));
            sql.push(")");
            Ok(())
        }
        "hasFacet" => {
            if from != QueryFrom::Resources {
                return Err(Error::Unsupported("hasFacet 过滤只支持 resources".into()));
            }
            let kind = value
                .get("kind")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::InvalidFilter("hasFacet 需要 kind".into()))?;
            sql.push(
                "EXISTS (SELECT 1 FROM g_facets f WHERE f.rid = g_resources.rid AND f.kind = ",
            );
            sql.text(kind.to_string());
            sql.push(" AND f.v <= ");
            sql.u64(versions.get("facets").copied().unwrap_or(0));
            match value.get("variant").and_then(Value::as_str) {
                Some(variant) => {
                    sql.push(" AND f.variant = ");
                    sql.text(variant.to_string());
                }
                None => {
                    sql.push(" AND f.variant = ''");
                }
            }
            sql.push(")");
            Ok(())
        }
        other => Err(Error::Unsupported(format!("过滤操作: {other}"))),
    }
}

fn order_clause(sql: &mut Sql, from: QueryFrom, order_by: &[OrderBy]) -> Result<()> {
    if order_by.is_empty() {
        sql.push(" ORDER BY seq ASC");
        return Ok(());
    }
    sql.push(" ORDER BY ");
    for (index, order) in order_by.iter().enumerate() {
        let column = column_of(from, &order.field)
            .ok_or_else(|| Error::UnknownField(order.field.clone()))?;
        if index > 0 {
            sql.push(", ");
        }
        sql.push(column);
        sql.push(if order.descending { " DESC" } else { " ASC" });
    }
    sql.push(", seq ASC");
    Ok(())
}

async fn base_rows(
    db: &Db,
    versions: &BTreeMap<String, u64>,
    request: &Request,
    limit: usize,
) -> Result<Vec<Base>> {
    let dialect = db.dialect();
    let mut sql = Sql::new(dialect);
    let table = table_of(request.from);
    sql.push("SELECT * FROM ").push(table).push(" WHERE v <= ");
    sql.u64(
        versions
            .get(version_key(request.from))
            .copied()
            .unwrap_or(0),
    );

    if let Some(expr) = &request.filter {
        sql.push(" AND ");
        compile(&mut sql, request.from, versions, expr)?;
    }
    if request.from == QueryFrom::Resources && !request.include_tombstoned {
        sql.push(
            " AND NOT EXISTS (SELECT 1 FROM g_resource_marks m WHERE m.rid = g_resources.rid AND m.v <= ",
        );
        sql.u64(versions.get("resource_marks").copied().unwrap_or(0));
        sql.push(
            " AND m.tombstone = 1 AND m.seq = (SELECT MAX(m2.seq) FROM g_resource_marks m2 WHERE m2.rid = m.rid AND m2.v <= ",
        );
        sql.u64(versions.get("resource_marks").copied().unwrap_or(0));
        sql.push("))");
    }
    if !request.has_facets.is_empty() {
        for selector in &request.has_facets {
            sql.push(
                " AND EXISTS (SELECT 1 FROM g_facets f WHERE f.rid = g_resources.rid AND f.kind = ",
            );
            sql.text(selector.kind.as_str().to_string());
            sql.push(" AND f.v <= ");
            sql.u64(versions.get("facets").copied().unwrap_or(0));
            match &selector.variant {
                Some(variant) => {
                    sql.push(" AND f.variant = ");
                    sql.text(variant.clone());
                }
                None => {
                    sql.push(" AND f.variant = ''");
                }
            }
            for field in &selector.fields {
                let column = match field.as_str() {
                    "text" => "body_text",
                    "struct" => "body_struct",
                    "blob" => "blob_ref",
                    other => return Err(Error::UnknownField(format!("facet 字段: {other}"))),
                };
                sql.push(" AND f.").push(column).push(" IS NOT NULL");
            }
            sql.push(")");
        }
    }
    if let Some(cursor) = request.cursor {
        sql.push(" AND seq > ");
        sql.u64(cursor);
    }
    order_clause(&mut sql, request.from, &request.order_by)?;
    sql.push(" LIMIT ");
    sql.u64(limit as u64 + 1);
    let rows = db.fetch(&sql).await?;

    let mut bases = Vec::with_capacity(rows.len());
    match request.from {
        QueryFrom::Resources => {
            let rids: Vec<String> = rows
                .iter()
                .map(|row| row.text("rid"))
                .collect::<Result<_>>()?;
            let labels = marks_for(db, versions, &rids).await?;
            for row in rows {
                let rid = row.text("rid")?;
                let seq = row.u64("seq")?;
                let mut payload = Map::new();
                payload.insert("rid".into(), Value::String(rid.clone()));
                payload.insert("ns".into(), Value::String(row.text("ns")?));
                if let Some(source) = row.opt_text("source_id")? {
                    payload.insert("sourceId".into(), Value::String(source));
                }
                payload.insert("createdAt".into(), Value::String(row.text("created_at")?));
                payload.insert("seq".into(), Value::from(seq));
                let mut label_map = Map::new();
                for label in labels.get(&rid).cloned().unwrap_or_default() {
                    if let Some((key, value)) = label.split_once('=') {
                        label_map.insert(key.to_string(), Value::String(value.to_string()));
                    }
                }
                payload.insert("labels".into(), Value::Object(label_map));
                bases.push(Base {
                    rid,
                    seq,
                    row: Value::Object(payload),
                });
            }
        }
        QueryFrom::Facets => {
            for row in rows {
                let rid = row.text("rid")?;
                let seq = row.u64("seq")?;
                bases.push(Base {
                    rid,
                    seq,
                    row: facet_payload(&row)?,
                });
            }
        }
        QueryFrom::Relations => {
            for row in rows {
                let rid = row.text("src_rid")?;
                let seq = row.u64("seq")?;
                bases.push(Base {
                    rid,
                    seq,
                    row: relation_payload(&row)?,
                });
            }
        }
    }
    Ok(bases)
}

async fn marks_for(
    db: &Db,
    versions: &BTreeMap<String, u64>,
    rids: &[String],
) -> Result<BTreeMap<String, Vec<String>>> {
    if rids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT rid, labels, seq FROM g_resource_marks WHERE v <= ");
    sql.u64(versions.get("resource_marks").copied().unwrap_or(0));
    sql.push(" AND rid IN (");
    for (index, rid) in rids.iter().enumerate() {
        if index > 0 {
            sql.push(", ");
        }
        sql.text(rid.clone());
    }
    sql.push(") ORDER BY seq ASC");
    let rows = db.fetch(&sql).await?;
    let mut heads = BTreeMap::new();
    for row in rows {
        heads.insert(
            row.text("rid")?,
            row.opt_text("labels")?.unwrap_or_default(),
        );
    }
    Ok(heads
        .into_iter()
        .map(|(rid, labels)| {
            let parsed: Vec<String> = serde_json::from_str(&labels).unwrap_or_default();
            (rid, parsed)
        })
        .collect())
}

fn facet_payload(row: &crate::db::Row) -> Result<Value> {
    let mut payload = Map::new();
    payload.insert("rid".into(), Value::String(row.text("rid")?));
    payload.insert("kind".into(), Value::String(row.text("kind")?));
    let variant = row.text("variant")?;
    if !variant.is_empty() {
        payload.insert("variant".into(), Value::String(variant));
    }
    payload.insert("version".into(), Value::from(row.u64("version")?));
    payload.insert(
        "contentHash".into(),
        Value::String(row.text("content_hash")?),
    );
    if let Some(media_type) = row.opt_text("media_type")? {
        payload.insert("mediaType".into(), Value::String(media_type));
    }
    payload.insert("size".into(), Value::from(row.u64("size")?));
    if let Some(text) = row.opt_text("body_text")? {
        payload.insert("bodyText".into(), Value::String(text));
    }
    if let Some(text) = row.opt_text("body_struct")? {
        payload.insert("bodyStruct".into(), Value::String(text));
    }
    if let Some(blob) = row.opt_text("blob_ref")? {
        payload.insert("blobRef".into(), Value::String(blob));
    }
    payload.insert("seq".into(), Value::from(row.u64("seq")?));
    Ok(Value::Object(payload))
}

fn relation_payload(row: &crate::db::Row) -> Result<Value> {
    let mut payload = Map::new();
    payload.insert("srcRid".into(), Value::String(row.text("src_rid")?));
    payload.insert("rel".into(), Value::String(row.text("rel")?));
    payload.insert("dstRid".into(), Value::String(row.text("dst_rid")?));
    payload.insert("seq".into(), Value::from(row.u64("seq")?));
    Ok(Value::Object(payload))
}

/// 已由最新一条标记写进墓碑的资源号。
async fn tombstoned_rids(
    db: &Db,
    versions: &BTreeMap<String, u64>,
    rids: &[String],
) -> Result<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    if rids.is_empty() {
        return Ok(out);
    }
    let marks = versions.get("resource_marks").copied().unwrap_or(0);
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT DISTINCT m.rid AS rid FROM g_resource_marks m WHERE m.v <= ");
    sql.u64(marks);
    sql.push(" AND m.tombstone = 1 AND m.seq = (SELECT MAX(m2.seq) FROM g_resource_marks m2 WHERE m2.rid = m.rid AND m2.v <= ");
    sql.u64(marks);
    sql.push(") AND m.rid IN (");
    for (index, rid) in rids.iter().enumerate() {
        if index > 0 {
            sql.push(", ");
        }
        sql.text(rid.clone());
    }
    sql.push(")");
    for row in db.fetch(&sql).await? {
        out.insert(row.text("rid")?);
    }
    Ok(out)
}

/// 按 rid 批量取资源行：检索命中的资源可能不在过滤结果里。
async fn resource_rows_by_rid(
    db: &Db,
    versions: &BTreeMap<String, u64>,
    rids: &[String],
) -> Result<Vec<Base>> {
    let mut sql = Sql::new(db.dialect());
    sql.push("SELECT rid, ns, source_id, created_at, seq FROM g_resources WHERE v <= ");
    sql.u64(versions.get("resources").copied().unwrap_or(0));
    sql.push(" AND rid IN (");
    for (index, rid) in rids.iter().enumerate() {
        if index > 0 {
            sql.push(", ");
        }
        sql.text(rid.clone());
    }
    sql.push(") ORDER BY seq ASC");
    let rows = db.fetch(&sql).await?;
    let mut bases = Vec::with_capacity(rows.len());
    for row in rows {
        let rid = row.text("rid")?;
        let seq = row.u64("seq")?;
        let mut payload = Map::new();
        payload.insert("rid".into(), Value::String(rid.clone()));
        payload.insert("ns".into(), Value::String(row.text("ns")?));
        if let Some(source) = row.opt_text("source_id")? {
            payload.insert("sourceId".into(), Value::String(source));
        }
        payload.insert("createdAt".into(), Value::String(row.text("created_at")?));
        payload.insert("seq".into(), Value::from(seq));
        bases.push(Base {
            rid,
            seq,
            row: Value::Object(payload),
        });
    }
    Ok(bases)
}
