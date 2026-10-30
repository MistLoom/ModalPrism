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
use crate::util::{self, cursor, page_total};
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
    offset: Option<String>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    source_id: Option<String>,
    /// 信源类型（feed/search/web/api/push/custom）；缺省或 `all` 不按类型过滤。
    #[serde(default, rename = "sourceType")]
    source_type: Option<String>,
    #[serde(default)]
    status: Option<String>,
    /// `all` 取消项目隔离过滤（管理与迁移用）。
    #[serde(default)]
    scope: Option<String>,
}

/// 筛选条件的公共构造：列表窗口与计数查询必须共享同一语义。
/// `allowed_sources` 为 Some 时按项目可见信源集合过滤（内容项目归属由其信源派生）。
fn content_where(
    source_id: Option<&str>,
    status: Option<&str>,
    allowed_sources: Option<&[String]>,
) -> Option<Expr> {
    let mut predicates = Vec::new();
    if let Some(source) = source_id {
        predicates.push(json!({"eq": ["sourceId", source]}));
    }
    if let Some(status) = status {
        predicates.push(json!({"eq": ["tombstone", status == "tombstoned"]}));
    }
    if let Some(sources) = allowed_sources {
        predicates.push(json!({"in": ["sourceId", sources]}));
    }
    let mut map = serde_json::Map::new();
    if predicates.len() == 1 {
        if let Value::Object(predicate) = &predicates[0] {
            map = predicate.clone();
        }
    } else if predicates.len() > 1 {
        map.insert("all".into(), Value::Array(predicates));
    }
    if map.is_empty() {
        None
    } else {
        serde_json::from_value(Value::Object(map)).ok()
    }
}

fn content_query(
    search: Option<&str>,
    source_id: Option<&str>,
    status: Option<&str>,
    allowed_sources: Option<&[String]>,
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

    let r#where = content_where(source_id, status, allowed_sources);

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

/// 总数计数上限：本地单机规模内精确；达到上限按上限返回（前端显示为封顶值）。
const COUNT_CAP: usize = 5_000;

/// 计数专用查询：与内容列表共享 where 语义，但只取 rid 并按单一 struct 变体过滤。
fn content_count_query(
    search: Option<&str>,
    source_id: Option<&str>,
    status: Option<&str>,
    allowed_sources: Option<&[String]>,
    variant: &str,
) -> StrataQuery {
    let select = vec![SelectItem::Field("rid".into())];
    let r#where = content_where(source_id, status, allowed_sources);
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
                variant: Some(variant.to_string()),
                k: COUNT_CAP as u32,
            }),
            as_of: None,
            order_by: Vec::new(),
            limit: Some(COUNT_CAP as u32),
            cursor: None,
            include_tombstoned: status == Some("tombstoned"),
        };
    }
    StrataQuery {
        from: QueryFrom::Resources,
        r#where,
        has_facets: vec![FacetSelector {
            kind: FacetKind::Struct,
            variant: Some(variant.into()),
            fields: Vec::new(),
        }],
        select,
        expand: Vec::new(),
        search: None,
        as_of: None,
        order_by: Vec::new(),
        limit: Some(COUNT_CAP as u32),
        cursor: None,
        include_tombstoned: status == Some("tombstoned"),
    }
}

/// 内容总数：文章 / 热榜 / 发现三种变体分别计数后按 rid 去重合并（与列表的事后过滤同语义）。
pub(crate) async fn item_count(
    state: &AppState,
    search: Option<&str>,
    source: Option<&str>,
    status: Option<&str>,
    allowed_sources: Option<&[String]>,
) -> Result<usize, ApiError> {
    let mut ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for variant in ["article", "hotlist", "discovery"] {
        let result = state
            .services
            .strata()
            .query(content_count_query(
                search,
                source,
                status,
                allowed_sources,
                variant,
            ))
            .await
            .map_err(ApiError::from)?;
        for row in result.rows {
            if let Some(rid) = row.get("rid").and_then(Value::as_str) {
                ids.insert(rid.to_string());
            }
        }
        if ids.len() >= COUNT_CAP {
            return Ok(COUNT_CAP);
        }
    }
    Ok(ids.len())
}

/// 按内容形态分别计数（不做跨形态去重），供项目范围的概览展示。
pub(crate) async fn variant_counts(
    state: &AppState,
    allowed_sources: Option<&[String]>,
) -> Result<Vec<Value>, ApiError> {
    let mut items = Vec::new();
    for variant in ["article", "hotlist", "discovery"] {
        let result = state
            .services
            .strata()
            .query(content_count_query(None, None, None, allowed_sources, variant))
            .await
            .map_err(ApiError::from)?;
        let mut ids: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for row in &result.rows {
            if let Some(rid) = row.get("rid").and_then(Value::as_str) {
                ids.insert(rid);
            }
        }
        if !ids.is_empty() {
            items.push(json!({"variant": variant, "total": ids.len()}));
        }
    }
    Ok(items)
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
    project: crate::ActiveProject,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    // 类型过滤：把类型解析为信源集合，再与项目可见集合取交；内容归属只能经信源派生。
    let type_ids = match query
        .source_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "all")
    {
        Some(kind) => {
            if config::source_type::SourceType::parse(kind).is_none() {
                return Err(ApiError::validation(
                    "sourceType",
                    "closedSet",
                    format!(
                        "未知类型: {kind}（应为 {}）",
                        config::source_type::SourceType::closed_set_text()
                    ),
                ));
            }
            Some(crate::catalog::source_ids_of_type(&state, kind).await?)
        }
        None => None,
    };
    // 项目隔离：内容归属由自有信源派生；共享信源只放开信源声明，不把内容带到其他项目。
    let allowed_sources: Option<Vec<String>> = if query.scope.as_deref() == Some("all") {
        None
    } else {
        let mut ids: Vec<String> = crate::catalog::owned_source_ids(&state, project.id())
            .await?
            .into_iter()
            .collect();
        ids.sort();
        Some(ids)
    };
    let allowed_sources = match (allowed_sources, type_ids) {
        (Some(visible), Some(typed)) => Some(
            visible
                .into_iter()
                .filter(|id| typed.contains(id))
                .collect::<Vec<_>>(),
        ),
        (None, Some(typed)) => {
            let mut ids: Vec<String> = typed.into_iter().collect();
            ids.sort();
            Some(ids)
        }
        (allowed, None) => allowed,
    };
    if allowed_sources.as_ref().is_some_and(|ids| ids.is_empty()) {
        return Ok(util::ok(&page_total(vec![], None, 0)));
    }
    let filter = format!(
        "content:{}:{}:{}:{}:{}",
        query.query.clone().unwrap_or_default(),
        query.source_id.clone().unwrap_or_default(),
        query.status.clone().unwrap_or_default(),
        query.scope.clone().unwrap_or_default(),
        allowed_sources
            .as_ref()
            .map(|v| v.join(","))
            .unwrap_or_default(),
    );
    let offset = util::page_offset(query.offset.as_deref(), query.cursor.as_deref(), &filter)?;
    let search = query
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let status = query
        .status
        .as_deref()
        .filter(|value| !value.is_empty() && *value != "all");
    // 内容行按 `source/<slug>` 存归属；接口同时接受裸 slug 与带前缀两种写法。
    let source = query
        .source_id
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(|value| {
            if value.starts_with("source/") {
                value.to_string()
            } else {
                format!("source/{value}")
            }
        });
    let spec = content_query(
        search,
        source.as_deref(),
        status,
        allowed_sources.as_deref(),
        limit + offset + 1,
    );
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
    let window = rows.len();
    rows = rows.into_iter().skip(offset).take(limit + 1).collect();
    let more = rows.len() > limit;
    let items: Vec<Value> = rows.into_iter().take(limit).collect();
    let next = if more || offset + items.len() < window {
        Some(cursor(&filter, offset + limit))
    } else {
        None
    };
    let total = item_count(&state, search, source.as_deref(), status, allowed_sources.as_deref()).await?;
    Ok(util::ok(&page_total(items, next, total)))
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

async fn read_row(state: &AppState, rid: &Rid, project_id: &str) -> Result<Value, ApiError> {
    let row = row_of(state, rid)
        .await?
        .ok_or_else(|| ApiError::not_found(rid.as_str()))?;
    assert_content_visible(state, &row, project_id).await?;
    Ok(row)
}

/// 按 rid 取内容行，不做项目过滤；不存在返回 None。
async fn row_of(state: &AppState, rid: &Rid) -> Result<Option<Value>, ApiError> {
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
    let row = page
        .rows
        .into_iter()
        .find(|row| {
            row.get("struct:article").is_some()
                || row.get("struct:hotlist").is_some()
                || row.get("struct:discovery").is_some()
        });
    Ok(row)
}

/// 内容可见性门禁：项目隔离下，仅归属可见信源（本项目或共享）的内容可读；
/// 无法归属（无 sourceId 或信源已删）的历史内容仅默认项目可见。
async fn assert_content_visible(
    state: &AppState,
    row: &Value,
    project_id: &str,
) -> Result<(), ApiError> {
    let source_id = row["sourceId"].as_str().unwrap_or("");
    if source_id.is_empty() {
        return if project_id == crate::projects::DEFAULT_PROJECT_ID {
            Ok(())
        } else {
            Err(ApiError::not_found("内容"))
        };
    }
    // 内容行里的 sourceId 可能是 collect 侧的 source/<slug> 形式，信源清单用裸 slug，
    // 两侧归一再比较，否则跨项目切换后详情会被误判为不可见。
    let slug = source_id.strip_prefix("source/").unwrap_or(source_id);
    let rows = crate::catalog::source_rows(state).await?;
    match rows
        .iter()
        .find(|item| item["id"].as_str() == Some(slug))
    {
        Some(source) if crate::catalog::source_editable(source, project_id) => Ok(()),
        Some(_) => Err(ApiError::not_found("内容")),
        // 信源已删除：视为历史内容，仅默认项目可见。
        None => {
            if project_id == crate::projects::DEFAULT_PROJECT_ID {
                Ok(())
            } else {
                Err(ApiError::not_found("内容"))
            }
        }
    }
}

async fn get_item(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Path(content_id): Path<String>,
) -> Result<Response, ApiError> {
    let rid = rid_of(&content_id)?;
    let row = read_row(&state, &rid, project.id()).await?;
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

/// 投递用的内容快照：标题、摘要、来源与正文摘录。项目可见性沿用详情同一门禁。
pub(crate) async fn delivery_snapshot(
    state: &AppState,
    content_id: &str,
    project_id: &str,
) -> Result<Value, ApiError> {
    let rid = rid_of(content_id)?;
    let row = read_row(state, &rid, project_id).await?;
    let names = source_names(state).await?;
    let item = item_view(&row, &names);
    let body = body_of(state, &rid).await?;
    let text = body
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            body.get("value")
                .filter(|value| !value.is_null())
                .map(|value| value.to_string())
        })
        .unwrap_or_default();
    Ok(json!({
        "contentId": content_id,
        "title": item["title"],
        "summary": item["summary"],
        "sourceId": item["sourceId"],
        "sourceName": item["sourceName"],
        "url": item.pointer("/struct/url").cloned().unwrap_or(Value::Null),
        "text": text.chars().take(8000).collect::<String>(),
    }))
}

async fn get_body(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Path(content_id): Path<String>,
) -> Result<Response, ApiError> {
    let rid = rid_of(&content_id)?;
    read_row(&state, &rid, project.id()).await?;
    let body = body_of(&state, &rid).await?;
    Ok(util::ok(&json!({"contentId": content_id, "body": body})))
}

async fn list_versions(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Path(content_id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let rid = rid_of(&content_id)?;
    read_row(&state, &rid, project.id()).await?;
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
    let offset = util::page_offset(query.offset.as_deref(), query.cursor.as_deref(), &filter)?;
    let total = items.len();
    let items: Vec<Value> = items.into_iter().skip(offset).take(limit + 1).collect();
    let more = items.len() > limit;
    let items: Vec<Value> = items.into_iter().take(limit).collect();
    let next = if more || offset + items.len() < total {
        Some(cursor(&filter, offset + limit))
    } else {
        None
    };
    Ok(util::ok(&page_total(items, next, total)))
}

async fn get_lineage(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Path(content_id): Path<String>,
) -> Result<Response, ApiError> {
    let rid = rid_of(&content_id)?;
    read_row(&state, &rid, project.id()).await?;
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
    project: crate::ActiveProject,
    Path(content_id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let rid = rid_of(&content_id)?;
    read_row(&state, &rid, project.id()).await?;
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
    let offset = util::page_offset(query.offset.as_deref(), query.cursor.as_deref(), &filter)?;
    let total = items.len();
    let items: Vec<Value> = items.into_iter().skip(offset).take(limit + 1).collect();
    let more = items.len() > limit;
    let items: Vec<Value> = items.into_iter().take(limit).collect();
    let next = if more || offset + items.len() < total {
        Some(cursor(&filter, offset + limit))
    } else {
        None
    };
    Ok(util::ok(&page_total(items, next, total)))
}

async fn patch_item(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Path(content_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["tags", "title"])?;
    let rid = rid_of(&content_id)?;
    let row = read_row(&state, &rid, project.id()).await?;
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
    let row = read_row(&state, &rid, project.id()).await?;
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
    project: crate::ActiveProject,
    Path(content_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let rid = rid_of(&content_id)?;
    let row = read_row(&state, &rid, project.id()).await?;
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
    project: crate::ActiveProject,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["source", "before", "after", "dryRun"])?;
    let requested = util::optional_text(map, "source")?.filter(|value| !value.trim().is_empty());
    let before = content_time(util::optional_text(map, "before")?, "before")?;
    let after = content_time(util::optional_text(map, "after")?, "after")?;
    let dry_run = matches!(map.get("dryRun"), Some(Value::Bool(true)));
    // 项目隔离：不传来源时按当前项目自有信源逐个清空，绝不跨项目删除。
    let mut visible: Vec<String> = crate::catalog::owned_source_ids(&state, project.id())
        .await?
        .into_iter()
        .collect();
    visible.sort();
    let scopes: Vec<String> = match requested.clone() {
        Some(source) => {
            if !visible.iter().any(|id| id == &source) {
                return Err(ApiError::not_found(format!("信源 {source}")));
            }
            vec![source]
        }
        None => visible,
    };
    let mut matched = 0u64;
    let mut deleted = 0u64;
    let mut truncated = false;
    for source in scopes {
        let filter = assembly::ContentFilter {
            source: Some(source),
            before,
            after,
        };
        let report = state
            .services
            .content_delete(&filter, dry_run)
            .await
            .map_err(ApiError::from)?;
        matched = matched.saturating_add(report.matched);
        deleted = deleted.saturating_add(report.deleted);
        truncated |= report.truncated;
    }
    if !dry_run {
        crate::observe::append_audit(
            &state,
            "content.purge",
            "succeeded",
            "按筛选删除内容",
            json!({
                "source": requested,
                "matched": matched,
                "deleted": deleted,
                "truncated": truncated,
            }),
            None,
        )
        .await?;
    }
    Ok(util::ok(&json!({
        "matched": matched,
        "deleted": deleted,
        "truncated": truncated,
        "dryRun": dry_run,
    })))
}

async fn batch_items(
    State(state): State<Shared>,
    project: crate::ActiveProject,
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
    // 项目隔离：已存在的内容必须当前项目可见；不存在的 rid 交给作业层按无匹配处理。
    for id in &ids {
        let rid = rid_of(id)?;
        if let Some(row) = row_of(&state, &rid).await? {
            assert_content_visible(&state, &row, project.id()).await?;
        }
    }
    let action = util::text(map, "action")?;
    if !matches!(
        action.as_str(),
        "delete" | "addTags" | "removeTags"
    ) {
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

/// 已发布快照列表。
///
/// 快照是存储层的库级版本（整个数据集的一次提交），不属于任何项目的资源：
/// 项目层级是「项目 → 信源 → 内容/任务/计划」，快照在这条链之上。
/// 因此这里如实列出库快照，只读元数据；真正会写数据的是重放，重放必须在项目范围内封口。
async fn list_snapshots(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let filter = "snapshots";
    let offset = util::page_offset(query.offset.as_deref(), query.cursor.as_deref(), filter)?;
    let changes = state
        .services
        .strata()
        .changes(0, Some(2_000))
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
    Ok(util::ok(&page_total(items, next, total)))
}

async fn create_replay(
    State(state): State<Shared>,
    project: crate::ActiveProject,
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
    // 快照不存在就没有可回放的版本，直接拒掉，避免后台任务空跑。
    let exists = state
        .services
        .strata()
        .changes(0, Some(2_000))
        .await
        .map_err(ApiError::from)?
        .iter()
        .any(|change| change.snapshot == snapshot_value);
    if !exists {
        return Err(ApiError::not_found(format!("快照 {snapshot_value}")));
    }
    let operation = crate::operations::start(
        &state,
        "content.replay",
        vec![json!({"kind": "snapshot", "id": snapshot})],
        json!({
            "kind": "content.replay",
            "project": project.id(),
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
    // 项目隔离：重放只作用于本项目自有信源产出的内容。
    // 项目 → 信源 → 内容 是唯一的归属链，共享信源的内容仍归它所属的那个项目，
    // 所以这里不能用“可见信源”口径，必须用自有口径，否则会跨项目回写。
    let project_id = action["project"]
        .as_str()
        .unwrap_or(crate::projects::DEFAULT_PROJECT_ID);
    let mut allowed: Vec<String> = crate::catalog::owned_source_ids(state, project_id)
        .await?
        .into_iter()
        .collect();
    allowed.sort();
    if allowed.is_empty() {
        return Ok(json!({
            "partial": false,
            "snapshot": snapshot,
            "scope": scope,
            "applied": 0,
            "skipped": 0,
            "note": "当前项目没有自有信源，未重放任何内容。",
        }));
    }
    let spec = StrataQuery {
        from: QueryFrom::Resources,
        r#where: content_where(None, None, Some(&allowed)),
        has_facets: Vec::new(),
        select: vec![
            SelectItem::Field("rid".into()),
            SelectItem::Field("sourceId".into()),
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
        // 双保险：即便查询层没跟上，也不会把其他项目的内容写回去。
        let row_source = row["sourceId"].as_str().unwrap_or("");
        if !allowed.iter().any(|id| id == row_source) {
            continue;
        }
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
        json!({"snapshot": snapshot, "project": project_id, "applied": applied, "skipped": skipped}),
        crate::observe::Refs::op(operation_id),
    )
    .await?;
    Ok(json!({
        "partial": false,
        "snapshot": snapshot,
        "scope": scope,
        "applied": applied,
        "skipped": skipped,
        "note": "按快照重放当前项目自有信源的结构化字段；未通过的项保留原版本。",
    }))
}
