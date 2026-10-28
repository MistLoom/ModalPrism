//! 内容领域资源：列表、正文、版本、血缘、关系、删除与快照。

use std::collections::BTreeMap;

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Value, json};
use strata_model::{
    ApplyOpts, AsOf, Body, ChangeSet, EdgeDir, Expr, FacetKind, FacetRef, FacetSelector,
    LineageDir, LineageInput, Op, OrderBy, Producer, Query as StrataQuery, QueryFrom, Rid, Search,
    SelectItem, Target, VersionSel,
};

use crate::error::ApiError;
use crate::operations::accepted;
use crate::util::{self, cursor, cursor_offset, page};
use crate::{AppState, Shared, new_id};

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/content/items", get(list_items))
        .route(
            "/content/items/{contentId}",
            get(get_item).patch(patch_item).delete(delete_item),
        )
        .route("/content/items/{contentId}/body", get(get_body))
        .route("/content/items/{contentId}/versions", get(list_versions))
        .route("/content/items/{contentId}/lineage", get(get_lineage))
        .route("/content/items/{contentId}/relations", get(list_relations))
        .route("/content/items/batch", post(batch_items))
        .route("/content/items/purge", post(purge_items))
        .route("/content/snapshots", get(list_snapshots))
        .route("/content/replays", post(create_replay))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListQuery {
    #[serde(default)]
    limit: Option<String>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    source_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

fn content_query(
    search: Option<&str>,
    source_id: Option<&str>,
    status: Option<&str>,
    limit: usize,
) -> StrataQuery {
    let mut has_facets = Vec::new();
    let mut select = vec![
        SelectItem::Field("rid".into()),
        SelectItem::Field("seq".into()),
        SelectItem::Field("tombstone".into()),
        SelectItem::Field("labels".into()),
        SelectItem::Field("sourceId".into()),
        SelectItem::Field("createdAt".into()),
    ];
    select.push(SelectItem::Facet(FacetSelector {
        kind: FacetKind::Struct,
        variant: Some("article".into()),
        fields: Vec::new(),
    }));
    select.push(SelectItem::Facet(FacetSelector {
        kind: FacetKind::Struct,
        variant: Some("hotlist".into()),
        fields: Vec::new(),
    }));
    select.push(SelectItem::Facet(FacetSelector {
        kind: FacetKind::Struct,
        variant: Some("discovery".into()),
        fields: Vec::new(),
    }));

    let mut predicates = Vec::new();
    if let Some(source) = source_id {
        predicates.push(json!({"eq": ["sourceId", source]}));
    }
    if let Some(status) = status {
        predicates.push(json!({"eq": ["tombstone", status == "tombstoned"]}));
    }
    let mut map = serde_json::Map::new();
    if predicates.len() == 1 {
        if let Value::Object(predicate) = &predicates[0] {
            map = predicate.clone();
        }
    } else if predicates.len() > 1 {
        map.insert("all".into(), Value::Array(predicates));
    }
    let r#where = if map.is_empty() {
        None
    } else {
        serde_json::from_value(Value::Object(map)).ok()
    };

    if let Some(text) = search {
        return StrataQuery {
            from: QueryFrom::Resources,
            r#where,
            has_facets: Vec::new(),
            select,
            expand: Vec::new(),
            search: Some(Search {
                text: Some(text.to_string()),
                vector: None,
                variant: None,
                k: limit as u32,
            }),
            as_of: None,
            order_by: Vec::new(),
            limit: Some(limit as u32),
            cursor: None,
            include_tombstoned: status == Some("tombstoned"),
        };
    }
    has_facets.push(FacetSelector {
        kind: FacetKind::Struct,
        variant: Some("article".into()),
        fields: Vec::new(),
    });
    // 文章与热榜二选一：有任一结构面即算内容；这里用查询后过滤，避免 AND 语义。
    has_facets.clear();
    StrataQuery {
        from: QueryFrom::Resources,
        r#where,
        has_facets,
        select,
        expand: Vec::new(),
        search: None,
        as_of: None,
        order_by: vec![OrderBy {
            field: "seq".into(),
            descending: true,
        }],
        limit: Some(limit as u32),
        cursor: None,
        include_tombstoned: status == Some("tombstoned"),
    }
}

async fn source_names(state: &AppState) -> Result<BTreeMap<String, String>, ApiError> {
    let mut names = BTreeMap::new();
    for entry in config::entries(&state.config_dir, "source")? {
        let name = entry
            .doc
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(&entry.slug)
            .to_string();
        names.insert(entry.slug.clone(), name.clone());
        names.insert(format!("source/{}", entry.slug), name.clone());
        if let Some(url) = entry
            .doc
            .pointer("/access/locator/url")
            .and_then(Value::as_str)
            && let Ok(parsed) = url::Url::parse(url)
            && let Some(host) = parsed.host_str()
        {
            names.insert(
                format!("host:{host}"),
                format!("{}{}{}", entry.slug, '\u{1f}', name),
            );
        }
    }
    Ok(names)
}

fn source_of(
    row: &Value,
    struct_value: Option<&Value>,
    names: &BTreeMap<String, String>,
) -> (String, String) {
    let direct = row["sourceId"]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            let labels = row["labels"].as_object()?;
            labels
                .get("source")
                .or_else(|| labels.get("sourceId"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    if let Some(source) = direct {
        let name = names
            .get(&source)
            .cloned()
            .unwrap_or_else(|| source.clone());
        return (source, name);
    }
    if let Some(url) = struct_value
        .and_then(|value| value.pointer("/locator/url"))
        .and_then(Value::as_str)
        .and_then(|value| url::Url::parse(value).ok())
        .and_then(|parsed| parsed.host_str().map(str::to_string))
        && let Some(indexed) = names.get(&format!("host:{url}"))
        && let Some((slug, name)) = indexed.split_once('\u{1f}')
    {
        return (slug.to_string(), name.to_string());
    }
    (String::new(), String::new())
}

fn struct_variant(row: &Value) -> &'static str {
    if row.get("struct:article").is_some() {
        "article"
    } else if row.get("struct:hotlist").is_some() {
        "hotlist"
    } else if row.get("struct:discovery").is_some() {
        "discovery"
    } else {
        "article"
    }
}

fn item_view(row: &Value, names: &BTreeMap<String, String>) -> Value {
    let struct_article = row.pointer("/struct:article/value");
    let struct_hotlist = row.pointer("/struct:hotlist/value");
    let struct_discovery = row.pointer("/struct:discovery/value");
    let (kind, struct_value) = if struct_article.is_some() {
        ("article", struct_article)
    } else if struct_hotlist.is_some() {
        ("hotlist", struct_hotlist)
    } else if struct_discovery.is_some() {
        ("discovery", struct_discovery)
    } else {
        ("structured", None)
    };
    let title = struct_value
        .and_then(|value| {
            value
                .pointer("/hints/title")
                .or_else(|| value.get("title"))
                .or_else(|| value.get("name"))
        })
        .and_then(Value::as_str)
        .unwrap_or("未命名内容");
    let published = struct_value
        .and_then(|value| {
            value
                .pointer("/hints/publishedAt")
                .or_else(|| value.get("publishedAt"))
        })
        .and_then(Value::as_str);
    let summary = struct_value
        .and_then(|value| {
            value
                .pointer("/hints/summary")
                .or_else(|| value.get("summary"))
        })
        .and_then(Value::as_str);
    let (source_id, source_name) = source_of(row, struct_value, names);
    let labels = row["labels"].as_object().cloned().unwrap_or_default();
    let tags: Vec<String> = labels
        .get("tags")
        .and_then(Value::as_str)
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|tag| !tag.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let tombstone = row["tombstone"].as_bool().unwrap_or(false);
    let etag = util::etag(&[
        "content",
        row["rid"].as_str().unwrap_or_default(),
        &row["seq"].to_string(),
        if tombstone { "tombstoned" } else { "active" },
    ]);
    json!({
        "id": row["rid"],
        "kind": kind,
        "title": title,
        "summary": summary,
        "sourceId": source_id,
        "sourceName": source_name,
        "status": if tombstone { "tombstoned" } else { "active" },
        "tags": tags,
        "publishedAt": published,
        "createdAt": row["createdAt"],
        "struct": struct_value.cloned().unwrap_or(Value::Null),
        "etag": etag,
    })
}

async fn list_items(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let filter = format!(
        "content:{}:{}:{}",
        query.query.clone().unwrap_or_default(),
        query.source_id.clone().unwrap_or_default(),
        query.status.clone().unwrap_or_default(),
    );
    let offset = cursor_offset(query.cursor.as_deref(), &filter)?;
    let search = query
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let status = query
        .status
        .as_deref()
        .filter(|value| !value.is_empty() && *value != "all");
    let source = query.source_id.as_deref().filter(|value| !value.is_empty());
    let spec = content_query(search, source, status, limit + offset + 1);
    let result = state
        .services
        .strata()
        .query(spec)
        .await
        .map_err(ApiError::from)?;
    let names = source_names(&state).await?;
    let mut rows: Vec<Value> = result
        .rows
        .iter()
        .filter(|row| {
            row.get("struct:article").is_some()
                || row.get("struct:hotlist").is_some()
                || row.get("struct:discovery").is_some()
        })
        .map(|row| item_view(row, &names))
        .collect();
    let total = rows.len();
    rows = rows.into_iter().skip(offset).take(limit + 1).collect();
    let more = rows.len() > limit;
    let items: Vec<Value> = rows.into_iter().take(limit).collect();
    let next = if more || offset + items.len() < total {
        Some(cursor(&filter, offset + limit))
    } else {
        None
    };
    Ok(util::ok(&page(items, next)))
}

fn rid_of(content_id: &str) -> Result<Rid, ApiError> {
    Rid::parse(content_id).map_err(|_| ApiError::validation("contentId", "format", "内容标识无效"))
}

fn facet_ref(rid: &Rid, kind: FacetKind, variant: Option<&str>) -> FacetRef {
    FacetRef {
        rid: rid.clone(),
        kind,
        variant: variant.map(str::to_string),
        version: VersionSel::Latest,
    }
}

async fn read_row(state: &AppState, rid: &Rid) -> Result<Value, ApiError> {
    let spec = StrataQuery {
        from: QueryFrom::Resources,
        r#where: serde_json::from_value::<Expr>(json!({"eq": ["rid", rid.as_str()]})).ok(),
        has_facets: Vec::new(),
        select: vec![
            SelectItem::Field("rid".into()),
            SelectItem::Field("seq".into()),
            SelectItem::Field("tombstone".into()),
            SelectItem::Field("labels".into()),
            SelectItem::Field("sourceId".into()),
            SelectItem::Field("createdAt".into()),
            SelectItem::Facet(FacetSelector {
                kind: FacetKind::Struct,
                variant: Some("article".into()),
                fields: Vec::new(),
            }),
            SelectItem::Facet(FacetSelector {
                kind: FacetKind::Struct,
                variant: Some("hotlist".into()),
                fields: Vec::new(),
            }),
            SelectItem::Facet(FacetSelector {
                kind: FacetKind::Struct,
                variant: Some("discovery".into()),
                fields: Vec::new(),
            }),
        ],
        expand: Vec::new(),
        search: None,
        as_of: None,
        order_by: Vec::new(),
        limit: Some(1),
        cursor: None,
        include_tombstoned: true,
    };
    let page = state
        .services
        .strata()
        .query(spec)
        .await
        .map_err(ApiError::from)?;
    page.rows
        .into_iter()
        .find(|row| {
            row.get("struct:article").is_some()
                || row.get("struct:hotlist").is_some()
                || row.get("struct:discovery").is_some()
        })
        .ok_or_else(|| ApiError::not_found(rid.as_str()))
}

async fn get_item(
    State(state): State<Shared>,
    Path(content_id): Path<String>,
) -> Result<Response, ApiError> {
    let rid = rid_of(&content_id)?;
    let row = read_row(&state, &rid).await?;
    let names = source_names(&state).await?;
    let mut item = item_view(&row, &names);
    let body = body_of(&state, &rid).await?;
    item["body"] = body;
    let kinds = state
        .services
        .strata()
        .kinds(&rid, true)
        .await
        .map_err(ApiError::from)?;
    item["facets"] = json!(
        kinds
            .iter()
            .map(|summary| json!({
                "kind": summary.kind.as_str(),
                "variant": summary.variant,
                "version": summary.latest,
                "versions": summary.versions,
                "mediaType": summary.media_type,
                "size": summary.size,
            }))
            .collect::<Vec<_>>()
    );
    let mut response = util::ok(&item);
    if let Ok(value) = item["etag"].as_str().unwrap_or("").parse() {
        response
            .headers_mut()
            .insert(axum::http::header::ETAG, value);
    }
    Ok(response)
}

async fn body_of(state: &AppState, rid: &Rid) -> Result<Value, ApiError> {
    for (kind, variant) in [
        (FacetKind::Md, Some("readable")),
        (FacetKind::Html, None),
        (FacetKind::Text, None),
        (FacetKind::Md, None),
    ] {
        let kind_text = kind.as_str().to_string();
        let reference = facet_ref(rid, kind, variant);
        if let Some(facet) = state
            .services
            .strata()
            .get(&reference, false)
            .await
            .map_err(ApiError::from)?
        {
            if let Some(text) = facet.text {
                return Ok(json!({
                    "kind": kind_text,
                    "variant": variant,
                    "version": facet.version,
                    "mediaType": facet.media_type,
                    "text": text,
                }));
            }
            if let Some(value) = facet.value {
                return Ok(json!({
                    "kind": kind_text,
                    "variant": variant,
                    "version": facet.version,
                    "value": value,
                }));
            }
        }
    }
    Ok(Value::Null)
}

async fn get_body(
    State(state): State<Shared>,
    Path(content_id): Path<String>,
) -> Result<Response, ApiError> {
    let rid = rid_of(&content_id)?;
    read_row(&state, &rid).await?;
    let body = body_of(&state, &rid).await?;
    Ok(util::ok(&json!({"contentId": content_id, "body": body})))
}

async fn list_versions(
    State(state): State<Shared>,
    Path(content_id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let rid = rid_of(&content_id)?;
    read_row(&state, &rid).await?;
    let kinds = state
        .services
        .strata()
        .kinds(&rid, true)
        .await
        .map_err(ApiError::from)?;
    let mut items = Vec::new();
    for summary in kinds {
        let reference = FacetRef {
            rid: rid.clone(),
            kind: summary.kind.clone(),
            variant: summary.variant.clone(),
            version: VersionSel::Latest,
        };
        let versions = state
            .services
            .strata()
            .history(&reference)
            .await
            .map_err(ApiError::from)?;
        for version in versions {
            items.push(json!({
                "id": format!("{}:{}:{}", content_id, summary.kind.as_str(), version.version),
                "kind": summary.kind.as_str(),
                "variant": summary.variant,
                "version": version.version,
                "contentHash": version.content_hash,
                "createdAt": version.created_at.to_rfc3339().unwrap_or_default(),
            }));
        }
    }
    items.sort_by(|left, right| {
        right["createdAt"]
            .as_str()
            .unwrap_or("")
            .cmp(left["createdAt"].as_str().unwrap_or(""))
    });
    let limit = util::limit(query.limit.as_deref())?;
    let filter = format!("versions:{content_id}");
    let offset = cursor_offset(query.cursor.as_deref(), &filter)?;
    let total = items.len();
    let items: Vec<Value> = items.into_iter().skip(offset).take(limit + 1).collect();
    let more = items.len() > limit;
    let items: Vec<Value> = items.into_iter().take(limit).collect();
    let next = if more || offset + items.len() < total {
        Some(cursor(&filter, offset + limit))
    } else {
        None
    };
    Ok(util::ok(&page(items, next)))
}

async fn get_lineage(
    State(state): State<Shared>,
    Path(content_id): Path<String>,
) -> Result<Response, ApiError> {
    let rid = rid_of(&content_id)?;
    read_row(&state, &rid).await?;
    let reference = facet_ref(&rid, FacetKind::Struct, Some("article"));
    let graph = match state
        .services
        .strata()
        .lineage(&reference, LineageDir::Up, 3)
        .await
    {
        Ok(graph) => graph,
        Err(_) => {
            let fallback = facet_ref(&rid, FacetKind::Struct, Some("discovery"));
            state
                .services
                .strata()
                .lineage(&fallback, LineageDir::Up, 3)
                .await
                .map_err(ApiError::from)?
        }
    };
    let nodes: Vec<Value> = graph
        .nodes
        .iter()
        .map(|node| {
            json!({
                "rid": node.facet.rid.as_str(),
                "kind": node.facet.kind.as_str(),
                "variant": node.facet.variant,
                "module": node.module,
                "version": node.version,
                "paramsHash": node.params_hash,
            })
        })
        .collect();
    let edges: Vec<Value> = graph
        .edges
        .iter()
        .map(|edge| {
            json!({
                "from": {"rid": edge.from.rid.as_str(), "kind": edge.from.kind.as_str(), "variant": edge.from.variant},
                "to": {"rid": edge.to.rid.as_str(), "kind": edge.to.kind.as_str(), "variant": edge.to.variant},
            })
        })
        .collect();
    Ok(util::ok(
        &json!({"contentId": content_id, "nodes": nodes, "edges": edges}),
    ))
}

async fn list_relations(
    State(state): State<Shared>,
    Path(content_id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let rid = rid_of(&content_id)?;
    read_row(&state, &rid).await?;
    let mut items = Vec::new();
    for direction in [EdgeDir::Out, EdgeDir::In] {
        let edges = state
            .services
            .strata()
            .related(&rid, None, direction)
            .await
            .map_err(ApiError::from)?;
        for edge in edges {
            items.push(json!({
                "id": format!("{}:{}:{}", edge.src.as_str(), edge.rel, edge.dst.as_str()),
                "direction": if direction == EdgeDir::Out { "out" } else { "in" },
                "relation": edge.rel,
                "sourceId": edge.src.as_str(),
                "targetId": edge.dst.as_str(),
                "attrs": edge.attrs,
            }));
        }
    }
    let limit = util::limit(query.limit.as_deref())?;
    let filter = format!("relations:{content_id}");
    let offset = cursor_offset(query.cursor.as_deref(), &filter)?;
    let total = items.len();
    let items: Vec<Value> = items.into_iter().skip(offset).take(limit + 1).collect();
    let more = items.len() > limit;
    let items: Vec<Value> = items.into_iter().take(limit).collect();
    let next = if more || offset + items.len() < total {
        Some(cursor(&filter, offset + limit))
    } else {
        None
    };
    Ok(util::ok(&page(items, next)))
}

async fn patch_item(
    State(state): State<Shared>,
    Path(content_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["tags", "title"])?;
    let rid = rid_of(&content_id)?;
    let row = read_row(&state, &rid).await?;
    let names = source_names(&state).await?;
    let current = item_view(&row, &names);
    util::if_match(&headers, current["etag"].as_str().unwrap_or(""))?;
    if let Some(tags) = util::optional_strings(map, "tags")? {
        let mut set = BTreeMap::new();
        if tags.is_empty() {
            state
                .services
                .content_label(std::slice::from_ref(&rid), &set, &["tags".to_string()])
                .await
                .map_err(ApiError::from)?;
        } else {
            set.insert("tags".to_string(), tags.join(", "));
            state
                .services
                .content_label(std::slice::from_ref(&rid), &set, &[])
                .await
                .map_err(ApiError::from)?;
        }
        set.clear();
    }
    if let Some(title) = util::optional_text(map, "title")?
        && let Some(value) = current["struct"].as_object().cloned()
    {
        let variant = struct_variant(&row);
        let mut next = value;
        if let Some(hints) = next.get_mut("hints").and_then(Value::as_object_mut) {
            hints.insert("title".into(), Value::String(title.clone()));
        }
        next.insert("title".into(), Value::String(title));
        let ops = vec![Op::PutFacet {
            target: Target::Rid(rid.clone()),
            kind: FacetKind::Struct,
            variant: Some(variant.into()),
            body: Body::Struct {
                value: Value::Object(next),
            },
            media_type: Some("application/json".into()),
            lineage: LineageInput {
                inputs: Vec::new(),
                params_hash: "content-api/metadata".into(),
            },
        }];
        let changeset = ChangeSet {
            producer: Producer {
                module: "content-api".into(),
                version: "1".into(),
                config_generation: state.services.config_generation(),
            },
            idempotency_key: None,
            ack: None,
            ops,
        };
        state
            .services
            .strata()
            .apply(changeset, ApplyOpts::default())
            .await
            .map_err(ApiError::from)?;
        state
            .services
            .committer_flush()
            .await
            .map_err(ApiError::from)?;
    }
    crate::observe::append_audit(
        &state,
        "content.update",
        "succeeded",
        "修改内容元数据",
        json!({"contentId": content_id}),
        Some(crate::observe::Refs::content(&content_id)),
    )
    .await?;
    let row = read_row(&state, &rid).await?;
    let item = item_view(&row, &names);
    let mut response = util::ok(&item);
    if let Ok(value) = item["etag"].as_str().unwrap_or("").parse() {
        response
            .headers_mut()
            .insert(axum::http::header::ETAG, value);
    }
    Ok(response)
}

async fn delete_item(
    State(state): State<Shared>,
    Path(content_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let rid = rid_of(&content_id)?;
    let row = read_row(&state, &rid).await?;
    let names = source_names(&state).await?;
    let current = item_view(&row, &names);
    util::if_match(&headers, current["etag"].as_str().unwrap_or(""))?;
    let operation = crate::operations::start(
        &state,
        "content.batch",
        vec![json!({"kind": "content", "id": content_id})],
        json!({"kind": "content.batch", "ids": [content_id], "action": "delete"}),
    )
    .await?;
    Ok(accepted(&operation))
}

fn content_time(
    value: Option<String>,
    field: &str,
) -> Result<Option<assembly::domain::Timestamp>, ApiError> {
    let Some(text) = value.filter(|text| !text.trim().is_empty()) else {
        return Ok(None);
    };
    assembly::domain::Timestamp::parse(text.trim())
        .map(Some)
        .map_err(|_| ApiError::validation(field, "format", "时间需为 RFC3339"))
}

/// 按筛选物理删除内容；单批上限由存储层控制，`truncated` 为真时继续调用。
async fn purge_items(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["source", "before", "after", "dryRun"])?;
    let filter = assembly::ContentFilter {
        source: util::optional_text(map, "source")?.filter(|value| !value.trim().is_empty()),
        before: content_time(util::optional_text(map, "before")?, "before")?,
        after: content_time(util::optional_text(map, "after")?, "after")?,
    };
    let dry_run = matches!(map.get("dryRun"), Some(Value::Bool(true)));
    let report = state
        .services
        .content_delete(&filter, dry_run)
        .await
        .map_err(ApiError::from)?;
    if !dry_run {
        crate::observe::append_audit(
            &state,
            "content.purge",
            "succeeded",
            "按筛选删除内容",
            json!({
                "source": filter.source,
                "matched": report.matched,
                "deleted": report.deleted,
                "truncated": report.truncated,
            }),
            None,
        )
        .await?;
    }
    Ok(util::ok(&json!({
        "matched": report.matched,
        "deleted": report.deleted,
        "truncated": report.truncated,
        "dryRun": report.dry_run,
    })))
}

async fn batch_items(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["ids", "action", "tags"])?;
    let ids = util::strings(map, "ids")?;
    if ids.is_empty() || ids.len() > 1000 {
        return Err(ApiError::validation(
            "ids",
            "outOfRange",
            "数量需要在 1 到 1000 之间",
        ));
    }
    let action = util::text(map, "action")?;
    if !matches!(action.as_str(), "delete" | "addTags" | "removeTags") {
        return Err(ApiError::validation(
            "action",
            "enum",
            "内容批量动作不支持该值",
        ));
    }
    let tags = util::optional_strings(map, "tags")?.unwrap_or_default();
    if matches!(action.as_str(), "addTags" | "removeTags") && tags.is_empty() {
        return Err(ApiError::validation(
            "tags",
            "required",
            "打标动作需要 tags",
        ));
    }
    let operation = crate::operations::start(
        &state,
        "content.batch",
        vec![json!({"kind": "contentBatch", "id": new_id("batch")})],
        json!({"kind": "content.batch", "ids": ids, "action": action, "tags": tags}),
    )
    .await?;
    Ok(accepted(&operation))
}

pub async fn execute_batch(
    state: &AppState,
    operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let ids = action["ids"].as_array().cloned().unwrap_or_default();
    let decision = action["action"].as_str().unwrap_or("delete");
    let tags = action["tags"].as_array().cloned().unwrap_or_default();
    let mut succeeded = Vec::new();
    let mut failed = Vec::new();
    for id in ids {
        let Some(id) = id.as_str() else { continue };
        let Ok(rid) = Rid::parse(id) else {
            failed.push(json!({"id": id, "code": "validationFailed", "message": "内容标识无效"}));
            continue;
        };
        let outcome = match decision {
            "delete" => state
                .services
                .content_delete_rids(std::slice::from_ref(&rid))
                .await
                .map(|_| ()),
            "addTags" => {
                let mut set = BTreeMap::new();
                set.insert(
                    "tags".into(),
                    tags.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                state
                    .services
                    .content_label(std::slice::from_ref(&rid), &set, &[])
                    .await
                    .map(|_| ())
            }
            "removeTags" => {
                let unset: Vec<String> = tags
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                state
                    .services
                    .content_label(std::slice::from_ref(&rid), &BTreeMap::new(), &unset)
                    .await
                    .map(|_| ())
            }
            _ => Err(assembly::Error::new("不支持的内容动作")),
        };
        match outcome {
            Ok(()) => succeeded.push(json!({"id": id})),
            Err(error) => failed.push(json!({
                "id": id,
                "code": "conflict",
                "message": error.to_string(),
            })),
        }
    }
    crate::observe::append_audit(
        state,
        "content.batch",
        if failed.is_empty() {
            "succeeded"
        } else {
            "partiallySucceeded"
        },
        "批量内容动作",
        json!({"action": decision, "succeeded": succeeded.len(), "failed": failed.len()}),
        Some(crate::observe::Refs::op(operation_id)),
    )
    .await?;
    Ok(json!({
        "partial": !failed.is_empty() && !succeeded.is_empty(),
        "succeeded": succeeded,
        "failed": failed,
    }))
}

async fn list_snapshots(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let filter = "snapshots";
    let offset = cursor_offset(query.cursor.as_deref(), filter)?;
    let changes = state
        .services
        .strata()
        .changes(0, Some(200))
        .await
        .map_err(ApiError::from)?;
    let mut items: Vec<Value> = changes
        .into_iter()
        .map(|change| {
            json!({
                "id": change.snapshot.to_string(),
                "snapshot": change.snapshot,
                "publishedAt": change.published_at.to_rfc3339().unwrap_or_default(),
                "commitCount": change.commit_ids.len(),
                "commits": change.commit_ids.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
            })
        })
        .collect();
    items.sort_by(|left, right| right["snapshot"].as_i64().cmp(&left["snapshot"].as_i64()));
    let total = items.len();
    let items: Vec<Value> = items.into_iter().skip(offset).take(limit + 1).collect();
    let more = items.len() > limit;
    let items: Vec<Value> = items.into_iter().take(limit).collect();
    let next = if more || offset + items.len() < total {
        Some(cursor(filter, offset + limit))
    } else {
        None
    };
    Ok(util::ok(&page(items, next)))
}

async fn create_replay(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["snapshotId", "scope", "contentIds"])?;
    let snapshot = util::text(map, "snapshotId")?;
    let scope = util::text(map, "scope")?;
    if !matches!(scope.as_str(), "all" | "failed" | "selected") {
        return Err(ApiError::validation(
            "scope",
            "enum",
            "只支持 all、failed 或 selected",
        ));
    }
    let content_ids = util::optional_strings(map, "contentIds")?.unwrap_or_default();
    if scope == "selected" && content_ids.is_empty() {
        return Err(ApiError::validation(
            "contentIds",
            "required",
            "selected 需要 contentIds",
        ));
    }
    let snapshot_value = snapshot
        .parse::<u64>()
        .map_err(|_| ApiError::validation("snapshotId", "format", "快照标识无效"))?;
    let operation = crate::operations::start(
        &state,
        "content.replay",
        vec![json!({"kind": "snapshot", "id": snapshot})],
        json!({
            "kind": "content.replay",
            "snapshot": snapshot_value,
            "scope": scope,
            "contentIds": content_ids,
        }),
    )
    .await?;
    Ok(accepted(&operation))
}

pub async fn execute_replay(
    state: &AppState,
    operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let snapshot = action["snapshot"].as_u64().unwrap_or(0);
    let scope = action["scope"].as_str().unwrap_or("all");
    let selected: Vec<String> = action["contentIds"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect();
    let spec = StrataQuery {
        from: QueryFrom::Resources,
        r#where: None,
        has_facets: Vec::new(),
        select: vec![
            SelectItem::Field("rid".into()),
            SelectItem::Facet(FacetSelector {
                kind: FacetKind::Struct,
                variant: Some("article".into()),
                fields: Vec::new(),
            }),
            SelectItem::Facet(FacetSelector {
                kind: FacetKind::Struct,
                variant: Some("hotlist".into()),
                fields: Vec::new(),
            }),
            SelectItem::Facet(FacetSelector {
                kind: FacetKind::Struct,
                variant: Some("discovery".into()),
                fields: Vec::new(),
            }),
        ],
        expand: Vec::new(),
        search: None,
        as_of: Some(AsOf::Snapshot(snapshot)),
        order_by: Vec::new(),
        limit: Some(200),
        cursor: None,
        include_tombstoned: false,
    };
    let page = state
        .services
        .strata()
        .query(spec)
        .await
        .map_err(ApiError::from)?;
    let mut applied = 0usize;
    let mut skipped = 0usize;
    for row in page.rows {
        let Some(rid) = row["rid"].as_str().and_then(|value| Rid::parse(value).ok()) else {
            skipped += 1;
            continue;
        };
        if scope == "selected" && !selected.contains(&rid.as_str().to_string()) {
            continue;
        }
        let (variant, value) = if let Some(value) = row.pointer("/struct:article/value") {
            ("article", value.clone())
        } else if let Some(value) = row.pointer("/struct:hotlist/value") {
            ("hotlist", value.clone())
        } else if let Some(value) = row.pointer("/struct:discovery/value") {
            ("discovery", value.clone())
        } else {
            skipped += 1;
            continue;
        };
        let changeset = ChangeSet {
            producer: Producer {
                module: "content-replay".into(),
                version: "1".into(),
                config_generation: state.services.config_generation(),
            },
            idempotency_key: Some(format!("replay-{snapshot}-{}", rid.as_str())),
            ack: None,
            ops: vec![Op::PutFacet {
                target: Target::Rid(rid.clone()),
                kind: FacetKind::Struct,
                variant: Some(variant.into()),
                body: Body::Struct { value },
                media_type: Some("application/json".into()),
                lineage: LineageInput {
                    inputs: Vec::new(),
                    params_hash: format!("replay/{snapshot}"),
                },
            }],
        };
        state
            .services
            .strata()
            .apply(changeset, ApplyOpts::default())
            .await
            .map_err(ApiError::from)?;
        applied += 1;
        if crate::operations::check_cancel(state, operation_id)
            .await
            .is_err()
        {
            break;
        }
    }
    state
        .services
        .committer_flush()
        .await
        .map_err(ApiError::from)?;
    crate::observe::append_event(
        state,
        "content.replay",
        "快照重放完成",
        json!({"snapshot": snapshot, "applied": applied, "skipped": skipped}),
        crate::observe::Refs::op(operation_id),
    )
    .await?;
    Ok(json!({
        "partial": false,
        "snapshot": snapshot,
        "scope": scope,
        "applied": applied,
        "skipped": skipped,
        "note": "按快照重放结构化字段；未通过的项保留原版本。",
    }))
}
