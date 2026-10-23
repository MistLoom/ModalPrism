//! 在一份快照视图上执行查询：内存过滤（eq/neq/in/gt/gte/lt/lte/all）、
//! facet 存在性筛选、排序与 keyset 游标。

use std::cmp::Ordering;
use std::collections::HashSet;

use serde_json::{Map, Value};
use strata_gateway::{
    Dataset as View, FacetRow as FacetRecord, RelationRow as RelationRecord,
    ResourceRow as ResourceRecord,
};
use strata_model::{EdgeDir, Expand, Expr, FacetSelector, Query, QueryFrom, Timestamp};

use crate::Error;

pub(crate) struct Page {
    pub rows: Vec<Value>,
    pub next_cursor: Option<String>,
}

pub(crate) fn execute(view: &View, query: &Query) -> Result<Page, Error> {
    if query.search.is_some() {
        return Err(Error::QueryShape);
    }
    if query.cursor.is_some() && !query.order_by.is_empty() {
        return Err(Error::CursorConflict);
    }
    if (!query.has_facets.is_empty() || !query.expand.is_empty())
        && query.from != QueryFrom::Resources
    {
        return Err(Error::QueryShape);
    }
    let cursor = match &query.cursor {
        Some(text) => Some(text.parse::<u64>().map_err(|_| Error::InvalidCursor)?),
        None => None,
    };
    let mut rows = match query.from {
        QueryFrom::Resources => resource_rows(view)?,
        QueryFrom::Facets => facet_rows(view)?,
        QueryFrom::Relations => relation_rows(view)?,
    };
    if let Some(expr) = &query.r#where {
        let mut kept = Vec::new();
        for row in rows {
            if eval(expr, &row)? {
                kept.push(row);
            }
        }
        rows = kept;
    }
    if !query.has_facets.is_empty() {
        rows.retain(|row| {
            let Some(rid) = row.get("rid").and_then(Value::as_str) else {
                return false;
            };
            query
                .has_facets
                .iter()
                .all(|selector| has_facet(view, rid, selector))
        });
    }
    if let Some(cursor) = cursor {
        rows.retain(|row| row.get("seq").and_then(Value::as_u64).unwrap_or(0) > cursor);
    }
    if query.order_by.is_empty() {
        rows.sort_by_key(|row| row.get("seq").and_then(Value::as_u64).unwrap_or(0));
    } else {
        rows.sort_by(|left, right| compare_order(query, left, right).unwrap_or(Ordering::Equal));
        compare_order_checked(query, &rows)?;
    }
    let limit = query.limit.map(|limit| limit as usize);
    let next_cursor = match (query.order_by.is_empty(), limit) {
        (true, Some(limit)) if rows.len() > limit => rows.get(limit - 1).and_then(seq_text),
        (true, Some(limit)) if rows.len() == limit => rows.last().and_then(seq_text),
        _ => None,
    };
    if let Some(limit) = limit {
        rows.truncate(limit);
    }
    let mut projected = Vec::with_capacity(rows.len());
    for row in rows {
        projected.push(project(view, query, row)?);
    }
    Ok(Page {
        rows: projected,
        next_cursor,
    })
}

fn seq_text(row: &Value) -> Option<String> {
    row.get("seq")
        .and_then(Value::as_u64)
        .map(|seq| seq.to_string())
}

fn compare_order_checked(query: &Query, rows: &[Value]) -> Result<(), Error> {
    for pair in rows.windows(2) {
        compare_order(query, &pair[0], &pair[1])?;
    }
    Ok(())
}

fn compare_order(query: &Query, left: &Value, right: &Value) -> Result<Ordering, Error> {
    for order in &query.order_by {
        let cmp = cmp_field(
            &order.field,
            field(left, &order.field)?,
            field(right, &order.field)?,
        )?;
        let cmp = if order.descending { cmp.reverse() } else { cmp };
        if cmp != Ordering::Equal {
            return Ok(cmp);
        }
    }
    Ok(Ordering::Equal)
}

pub(crate) fn project(view: &View, query: &Query, row: Value) -> Result<Value, Error> {
    let Value::Object(base) = row else {
        return Err(Error::QueryShape);
    };
    let mut out = if query.select.is_empty() {
        base.clone()
    } else {
        let mut picked = Map::new();
        for item in &query.select {
            match item {
                strata_model::SelectItem::Field(name) => {
                    let value = base
                        .get(name)
                        .cloned()
                        .ok_or_else(|| Error::UnknownField(name.clone()))?;
                    picked.insert(name.clone(), value);
                }
                strata_model::SelectItem::Facet(selector) => {
                    if query.from != QueryFrom::Resources {
                        return Err(Error::QueryShape);
                    }
                    let Some(rid) = base.get("rid").and_then(Value::as_str) else {
                        return Err(Error::QueryShape);
                    };
                    if let Some(facet) = latest_facet(view, rid, selector) {
                        picked.insert(facet_name(selector), facet_value(facet, selector)?);
                    }
                }
            }
        }
        picked
    };
    if !query.expand.is_empty() {
        let Some(rid) = base.get("rid").and_then(Value::as_str) else {
            return Err(Error::QueryShape);
        };
        let expanded = expand_all(view, rid, &query.expand)?;
        if !expanded.is_empty() {
            out.insert("expand".into(), Value::Array(expanded));
        }
    }
    Ok(Value::Object(out))
}

fn expand_all(view: &View, rid: &str, items: &[Expand]) -> Result<Vec<Value>, Error> {
    let mut out = Vec::new();
    for item in items {
        let depth = item.depth.unwrap_or(1);
        if depth == 0 {
            continue;
        }
        let mut seen = HashSet::new();
        seen.insert(rid.to_string());
        let mut frontier = vec![rid.to_string()];
        for distance in 1..=depth {
            let mut next = Vec::new();
            for current in frontier {
                for relation in neighbors(view, &current, &item.rel, item.dir) {
                    let other = match item.dir {
                        EdgeDir::Out => relation.dst_rid.clone(),
                        EdgeDir::In => relation.src_rid.clone(),
                    };
                    if !seen.insert(other.clone()) {
                        continue;
                    }
                    out.push(expanded_row(view, item, distance, &other, relation));
                    next.push(other);
                }
            }
            frontier = next;
        }
    }
    Ok(out)
}

fn neighbors<'a>(view: &'a View, rid: &str, rel: &str, dir: EdgeDir) -> Vec<&'a RelationRecord> {
    view.relations
        .iter()
        .filter(|relation| relation.rel == rel)
        .filter(|relation| match dir {
            EdgeDir::Out => relation.src_rid == rid,
            EdgeDir::In => relation.dst_rid == rid,
        })
        .collect()
}

fn expanded_row(
    view: &View,
    item: &Expand,
    distance: u32,
    rid: &str,
    relation: &RelationRecord,
) -> Value {
    let mut row = view
        .resources
        .iter()
        .find(|resource| resource.rid == rid)
        .map(|resource| resource_value(view, resource))
        .unwrap_or_else(|| {
            let mut map = Map::new();
            map.insert("rid".into(), Value::String(rid.to_string()));
            Value::Object(map)
        });
    if let Value::Object(map) = &mut row {
        map.insert("rel".into(), Value::String(item.rel.clone()));
        map.insert(
            "dir".into(),
            Value::String(
                match item.dir {
                    EdgeDir::Out => "out",
                    EdgeDir::In => "in",
                }
                .into(),
            ),
        );
        map.insert("distance".into(), Value::from(distance));
        map.insert("commitId".into(), Value::String(relation.commit_id.clone()));
    }
    row
}

fn resource_rows(view: &View) -> Result<Vec<Value>, Error> {
    Ok(view
        .resources
        .iter()
        .map(|resource| resource_value(view, resource))
        .collect())
}

pub(crate) fn resource_value(view: &View, resource: &ResourceRecord) -> Value {
    let mark = latest_mark(view, &resource.rid);
    let labels = mark
        .map(|mark| label_object(&mark.labels))
        .unwrap_or_default();
    let tombstone = mark.is_some_and(|mark| mark.tombstone);
    let updated = updated_at(view, resource);
    serde_json::json!({
        "rid": resource.rid,
        "ns": resource.ns,
        "sourceId": resource.source_id,
        "createdAt": resource.created_at,
        "updatedAt": updated,
        "seq": resource.seq,
        "labels": labels,
        "tombstone": tombstone,
    })
}

fn label_object(labels: &[String]) -> Map<String, Value> {
    let mut map = Map::new();
    for item in labels {
        if let Some((key, value)) = item.split_once('=') {
            map.insert(key.to_string(), Value::String(value.to_string()));
        }
    }
    map
}

fn updated_at(view: &View, resource: &ResourceRecord) -> String {
    let mut best: Option<Timestamp> = Timestamp::parse(&resource.created_at).ok();
    let mut text = resource.created_at.clone();
    for facet in view.facets.iter().filter(|facet| facet.rid == resource.rid) {
        let Ok(created) = Timestamp::parse(&facet.created_at) else {
            continue;
        };
        if best.is_none_or(|current| created > current) {
            best = Some(created);
            text = facet.created_at.clone();
        }
    }
    text
}

fn latest_mark<'a>(view: &'a View, rid: &str) -> Option<&'a strata_gateway::MarkRow> {
    view.marks
        .iter()
        .filter(|mark| mark.rid == rid)
        .max_by_key(|mark| mark.seq)
}

fn facet_rows(view: &View) -> Result<Vec<Value>, Error> {
    Ok(view
        .facets
        .iter()
        .map(|facet| {
            serde_json::json!({
                "rid": facet.rid,
                "kind": facet.kind,
                "variant": facet.variant,
                "version": facet.version,
                "contentHash": facet.content_hash,
                "mediaType": facet.media_type,
                "size": facet.size,
                "blobRef": facet.blob_ref,
                "bodyText": facet.body_text,
                "bodyStruct": facet.body_struct,
                "createdAt": facet.created_at,
                "seq": facet.seq,
                "commitId": facet.commit_id,
            })
        })
        .collect())
}

fn relation_rows(view: &View) -> Result<Vec<Value>, Error> {
    Ok(view
        .relations
        .iter()
        .map(|relation| {
            serde_json::json!({
                "srcRid": relation.src_rid,
                "rel": relation.rel,
                "dstRid": relation.dst_rid,
                "attrs": relation.attrs,
                "commitId": relation.commit_id,
                "seq": relation.seq,
            })
        })
        .collect())
}

fn has_facet(view: &View, rid: &str, selector: &FacetSelector) -> bool {
    view.facets
        .iter()
        .any(|facet| facet.rid == rid && matches_selector(facet, selector))
}

fn matches_selector(facet: &FacetRecord, selector: &FacetSelector) -> bool {
    facet.kind == selector.kind.as_str()
        && match &selector.variant {
            Some(variant) => facet.variant.as_deref() == Some(variant.as_str()),
            None => true,
        }
}

fn latest_facet<'a>(
    view: &'a View,
    rid: &str,
    selector: &FacetSelector,
) -> Option<&'a FacetRecord> {
    view.facets
        .iter()
        .filter(|facet| facet.rid == rid && matches_selector(facet, selector))
        .max_by_key(|facet| facet.version)
}

fn facet_name(selector: &FacetSelector) -> String {
    match &selector.variant {
        Some(variant) => format!("{}:{variant}", selector.kind.as_str()),
        None => selector.kind.as_str().to_string(),
    }
}

fn facet_value(facet: &FacetRecord, selector: &FacetSelector) -> Result<Value, Error> {
    let mut value = Map::new();
    value.insert("version".into(), Value::from(facet.version));
    value.insert(
        "contentHash".into(),
        Value::String(facet.content_hash.clone()),
    );
    if let Some(media) = &facet.media_type {
        value.insert("mediaType".into(), Value::String(media.clone()));
    }
    if let Some(blob) = &facet.blob_ref {
        value.insert("blobRef".into(), Value::String(blob.clone()));
    }
    if let Some(text) = &facet.body_text {
        if !selector.fields.is_empty() {
            return Err(Error::UnknownField(selector.fields[0].clone()));
        }
        value.insert("text".into(), Value::String(text.clone()));
    }
    if let Some(raw) = &facet.body_struct {
        let parsed: Value = serde_json::from_str(raw).map_err(strata_gateway::Error::from)?;
        if selector.fields.is_empty() {
            value.insert("value".into(), parsed);
        } else {
            let object = parsed.as_object().ok_or(Error::QueryShape)?;
            let mut picked = Map::new();
            for field in &selector.fields {
                let item = object
                    .get(field)
                    .cloned()
                    .ok_or_else(|| Error::UnknownField(field.clone()))?;
                picked.insert(field.clone(), item);
            }
            value.insert("value".into(), Value::Object(picked));
        }
    }
    Ok(Value::Object(value))
}

fn eval(expr: &Expr, row: &Value) -> Result<bool, Error> {
    let map = expr.as_map();
    if map.len() != 1 {
        return Err(Error::QueryShape);
    }
    let (key, value) = map.iter().next().ok_or(Error::QueryShape)?;
    match key.as_str() {
        "all" => {
            let items = value.as_array().ok_or(Error::QueryShape)?;
            if items.is_empty() {
                return Err(Error::QueryShape);
            }
            for item in items {
                let nested = Expr::try_from(item.clone()).map_err(|_| Error::QueryShape)?;
                if !eval(&nested, row)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        "eq" => {
            let (field_name, expected) = pair(value)?;
            let actual = field(row, field_name)?;
            if matches!(field_name, "createdAt" | "updatedAt" | "publishedAt") {
                Ok(cmp_field(field_name, actual, expected)? == Ordering::Equal)
            } else {
                Ok(actual == expected)
            }
        }
        "neq" => {
            let (field_name, expected) = pair(value)?;
            let actual = field(row, field_name)?;
            if matches!(field_name, "createdAt" | "updatedAt" | "publishedAt") {
                Ok(cmp_field(field_name, actual, expected)? != Ordering::Equal)
            } else {
                Ok(actual != expected)
            }
        }
        "in" => {
            let (field_name, expected) = pair(value)?;
            let items = expected.as_array().ok_or(Error::QueryShape)?;
            if items.is_empty() {
                return Ok(false);
            }
            let actual = field(row, field_name)?;
            Ok(items.iter().any(|item| item == actual))
        }
        "gte" => {
            let (field_name, expected) = pair(value)?;
            Ok(cmp_field(field_name, field(row, field_name)?, expected)? != Ordering::Less)
        }
        "gt" => {
            let (field_name, expected) = pair(value)?;
            Ok(cmp_field(field_name, field(row, field_name)?, expected)? == Ordering::Greater)
        }
        "lte" => {
            let (field_name, expected) = pair(value)?;
            Ok(cmp_field(field_name, field(row, field_name)?, expected)? != Ordering::Greater)
        }
        "lt" => {
            let (field_name, expected) = pair(value)?;
            Ok(cmp_field(field_name, field(row, field_name)?, expected)? == Ordering::Less)
        }
        _ => Err(Error::UnsupportedExpr),
    }
}

fn pair(value: &Value) -> Result<(&str, &Value), Error> {
    let items = value.as_array().ok_or(Error::QueryShape)?;
    if items.len() != 2 {
        return Err(Error::QueryShape);
    }
    let name = items[0].as_str().ok_or(Error::QueryShape)?;
    Ok((name, &items[1]))
}

fn field<'a>(row: &'a Value, name: &str) -> Result<&'a Value, Error> {
    row.get(name)
        .ok_or_else(|| Error::UnknownField(name.to_string()))
}

fn cmp_field(name: &str, left: &Value, right: &Value) -> Result<Ordering, Error> {
    if matches!(name, "createdAt" | "updatedAt" | "publishedAt") {
        let left = timestamp_of(left)?;
        let right = timestamp_of(right)?;
        return Ok(left.cmp(&right));
    }
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => Ok(cmp_number(left, right)),
        (Value::String(left), Value::String(right)) => Ok(left.cmp(right)),
        (Value::Bool(left), Value::Bool(right)) => Ok(left.cmp(right)),
        _ => Err(Error::QueryShape),
    }
}

fn timestamp_of(value: &Value) -> Result<Timestamp, Error> {
    let text = value.as_str().ok_or(Error::QueryShape)?;
    Timestamp::parse(text)
        .map_err(strata_gateway::Error::from)
        .map_err(Error::from)
}

fn cmp_number(left: &serde_json::Number, right: &serde_json::Number) -> Ordering {
    if let (Some(left), Some(right)) = (left.as_i64(), right.as_i64()) {
        return left.cmp(&right);
    }
    if let (Some(left), Some(right)) = (left.as_u64(), right.as_u64()) {
        return left.cmp(&right);
    }
    left.as_f64()
        .unwrap_or(0.0)
        .partial_cmp(&right.as_f64().unwrap_or(0.0))
        .unwrap_or(Ordering::Equal)
}

#[cfg(test)]
mod tests {
    use super::eval;
    use serde_json::json;
    use strata_model::Expr;

    #[test]
    fn eq_matches_equivalent_timestamps() {
        let row = json!({
            "createdAt": "2020-01-01T00:00:00Z",
            "ns": "data"
        });
        let same = Expr::try_from(json!({
            "eq": ["createdAt", "2020-01-01T00:00:00+00:00"]
        }))
        .unwrap();
        assert!(eval(&same, &row).unwrap());
        let text = Expr::try_from(json!({"eq": ["ns", "data"]})).unwrap();
        assert!(eval(&text, &row).unwrap());
        let other = Expr::try_from(json!({"eq": ["ns", "other"]})).unwrap();
        assert!(!eval(&other, &row).unwrap());
    }

    #[test]
    fn range_and_set_operators() {
        let row = json!({
            "seq": 40,
            "sourceId": "source/openai",
            "createdAt": "2026-01-02T00:00:00Z",
        });
        let lt = Expr::try_from(json!({"lt": ["seq", 41]})).unwrap();
        assert!(eval(&lt, &row).unwrap());
        let lte = Expr::try_from(json!({"lte": ["seq", 40]})).unwrap();
        assert!(eval(&lte, &row).unwrap());
        let gt = Expr::try_from(json!({"gt": ["seq", 40]})).unwrap();
        assert!(!eval(&gt, &row).unwrap());
        let member =
            Expr::try_from(json!({"in": ["sourceId", ["source/openai", "source/mistral"]]}))
                .unwrap();
        assert!(eval(&member, &row).unwrap());
        let absent = Expr::try_from(json!({"in": ["sourceId", []]})).unwrap();
        assert!(!eval(&absent, &row).unwrap());
        let neq = Expr::try_from(json!({"neq": ["sourceId", "source/other"]})).unwrap();
        assert!(eval(&neq, &row).unwrap());
        let after = Expr::try_from(json!({"gte": ["createdAt", "2026-01-01T00:00:00Z"]})).unwrap();
        assert!(eval(&after, &row).unwrap());
        let before = Expr::try_from(json!({"lte": ["createdAt", "2025-12-31T00:00:00Z"]})).unwrap();
        assert!(!eval(&before, &row).unwrap());
    }

    #[test]
    fn all_combines_predicates() {
        let row = json!({"seq": 7, "sourceId": "s/a"});
        let combined = Expr::try_from(json!({
            "all": [
                {"lt": ["seq", 10]},
                {"eq": ["sourceId", "s/a"]},
            ]
        }))
        .unwrap();
        assert!(eval(&combined, &row).unwrap());
        let failing = Expr::try_from(json!({
            "all": [
                {"lt": ["seq", 10]},
                {"eq": ["sourceId", "s/b"]},
            ]
        }))
        .unwrap();
        assert!(!eval(&failing, &row).unwrap());
    }
}
