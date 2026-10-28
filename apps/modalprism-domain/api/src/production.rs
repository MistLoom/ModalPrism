//! 内容生产：InputSet、WorkflowVersion、Run、Attempt、Candidate、Review、Draft。

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::error::ApiError;
use crate::operations::accepted;
use crate::util::{self, cursor, cursor_offset, page};
use crate::{AppState, Shared, new_id, now};

const INPUTSET: &str = "input_set";
const WORKFLOW: &str = "workflow";
const VERSION: &str = "workflow_version";
const RUN: &str = "production_run";
const ATTEMPT: &str = "production_attempt";
const CANDIDATE: &str = "candidate";
const REVIEW: &str = "review";
const DRAFT: &str = "draft";
const UPLOAD: &str = "upload";

pub fn routes() -> Router<Shared> {
    Router::new()
        .route(
            "/production/inputSets",
            get(list_input_sets).post(create_input_set),
        )
        .route("/production/inputSets/upload", post(upload_input))
        .route("/production/inputSets/{inputSetId}", get(get_input_set))
        .route(
            "/production/workflows",
            get(list_workflows).post(create_workflow),
        )
        .route(
            "/production/workflows/{workflowId}/versions",
            get(list_versions).post(create_version),
        )
        .route("/production/runs", get(list_runs).post(create_run))
        .route("/production/runs/{runId}", get(get_run))
        .route("/production/runs/{runId}/attempts", get(list_attempts))
        .route("/production/runs/{runId}/retry", post(retry_run))
        .route("/production/candidates", get(list_candidates))
        .route(
            "/production/candidates/{candidateId}/reviews",
            post(review_candidate),
        )
        .route("/production/drafts", get(list_drafts).post(create_draft))
        .route(
            "/production/drafts/{draftId}",
            get(get_draft).patch(update_draft),
        )
}

#[derive(Debug, Clone, Deserialize)]
struct ListQuery {
    #[serde(default)]
    limit: Option<String>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

async fn list_docs(
    state: &AppState,
    kind: &str,
    query: ListQuery,
    status: Option<&str>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let filter = format!("{kind}:{}", status.unwrap_or(""));
    let offset = cursor_offset(query.cursor.as_deref(), &filter)?;
    let mut docs: Vec<Value> = state
        .store
        .list_documents(kind)
        .await?
        .into_iter()
        .map(|doc| {
            let mut body = doc.body;
            if let Some(map) = body.as_object_mut() {
                let tag = match kind {
                    WORKFLOW => Some(format!("workflow-{}-v{}", doc.id, doc.version)),
                    CANDIDATE => Some(format!("candidate-{}-v{}", doc.id, doc.version)),
                    DRAFT => Some(format!("draft-{}-v{}", doc.id, doc.version)),
                    _ => None,
                };
                if let Some(tag) = tag {
                    map.insert("etag".into(), Value::String(tag));
                }
            }
            body
        })
        .collect();
    if let Some(status) = status {
        docs.retain(|doc| doc["status"] == status);
    }
    docs.sort_by(|left, right| {
        right["createdAt"]
            .as_str()
            .unwrap_or("")
            .cmp(left["createdAt"].as_str().unwrap_or(""))
    });
    let total = docs.len();
    let items: Vec<Value> = docs.into_iter().skip(offset).take(limit + 1).collect();
    let more = items.len() > limit;
    let items: Vec<Value> = items.into_iter().take(limit).collect();
    let next = if more || offset + items.len() < total {
        Some(cursor(&filter, offset + limit))
    } else {
        None
    };
    Ok(util::ok(&page(items, next)))
}

async fn list_input_sets(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    list_docs(&state, INPUTSET, query, None).await
}

async fn get_input_set(
    State(state): State<Shared>,
    Path(input_set_id): Path<String>,
) -> Result<Response, ApiError> {
    let doc = state
        .store
        .get_document(INPUTSET, &input_set_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("输入集 {input_set_id}")))?;
    Ok(util::ok(&doc.body))
}

fn input_refs(items: &[Value]) -> Result<Vec<Value>, ApiError> {
    let mut out = Vec::new();
    for item in items {
        let Some(map) = item.as_object() else {
            return Err(ApiError::validation("inputs", "type", "输入项必须是对象"));
        };
        util::unknown(
            map,
            &[
                "kind",
                "contentId",
                "contentIds",
                "query",
                "text",
                "fileId",
                "url",
                "value",
            ],
        )?;
        let kind = util::text(map, "kind")?;
        match kind.as_str() {
            "content" => {
                let content_id = util::non_empty(map, "contentId")?;
                out.push(json!({"kind": "content", "contentId": content_id}));
            }
            "contentList" => {
                let content_ids = util::strings(map, "contentIds")?;
                if content_ids.is_empty() {
                    return Err(ApiError::validation("contentIds", "required", "不能为空"));
                }
                out.push(json!({"kind": "contentList", "contentIds": content_ids}));
            }
            "querySnapshot" => {
                let query = util::optional_object(map, "query")?
                    .ok_or_else(|| ApiError::validation("query", "required", "不能为空"))?;
                out.push(json!({"kind": "querySnapshot", "query": query}));
            }
            "text" => {
                let text = util::non_empty(map, "text")?;
                out.push(json!({"kind": "text", "text": text}));
            }
            "file" => {
                let file_id = util::non_empty(map, "fileId")?;
                out.push(json!({"kind": "file", "fileId": file_id}));
            }
            "url" => {
                let url = util::uri(map, "url")?;
                out.push(json!({"kind": "url", "url": url.to_string()}));
            }
            "structured" => {
                let value = map
                    .get("value")
                    .cloned()
                    .ok_or_else(|| ApiError::validation("value", "required", "不能为空"))?;
                out.push(json!({"kind": "structured", "value": value}));
            }
            _ => return Err(ApiError::validation("kind", "enum", "未知输入类型")),
        }
    }
    Ok(out)
}

async fn freeze_content(state: &AppState, rid: &str) -> Result<Value, ApiError> {
    let rid = strata_model::Rid::parse(rid)
        .map_err(|_| ApiError::validation("contentId", "format", "内容标识无效"))?;
    let spec = strata_model::Query {
        from: strata_model::QueryFrom::Resources,
        r#where: serde_json::from_value(json!({"eq": ["rid", rid.as_str()]})).ok(),
        has_facets: Vec::new(),
        select: vec![
            strata_model::SelectItem::Field("rid".into()),
            strata_model::SelectItem::Facet(strata_model::FacetSelector {
                kind: strata_model::FacetKind::Struct,
                variant: Some("article".into()),
                fields: Vec::new(),
            }),
            strata_model::SelectItem::Facet(strata_model::FacetSelector {
                kind: strata_model::FacetKind::Struct,
                variant: Some("hotlist".into()),
                fields: Vec::new(),
            }),
            strata_model::SelectItem::Facet(strata_model::FacetSelector {
                kind: strata_model::FacetKind::Struct,
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
        include_tombstoned: false,
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
        })
        .ok_or_else(|| ApiError::not_found(format!("内容 {rid}")))?;
    let value = row
        .pointer("/struct:article/value")
        .or_else(|| row.pointer("/struct:hotlist/value"))
        .or_else(|| row.pointer("/struct:discovery/value"))
        .cloned()
        .unwrap_or(Value::Null);
    let title = value
        .get("title")
        .or_else(|| value.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("未命名内容");
    Ok(json!({
        "kind": "content",
        "contentId": rid.as_str(),
        "title": title,
        "text": value.get("summary").and_then(Value::as_str).unwrap_or(""),
        "struct": value,
    }))
}

async fn freeze_query(
    state: &AppState,
    query: &serde_json::Map<String, Value>,
) -> Result<Value, ApiError> {
    let mut spec = strata_model::Query {
        from: strata_model::QueryFrom::Resources,
        r#where: None,
        has_facets: Vec::new(),
        select: vec![
            strata_model::SelectItem::Field("rid".into()),
            strata_model::SelectItem::Field("sourceId".into()),
            strata_model::SelectItem::Facet(strata_model::FacetSelector {
                kind: strata_model::FacetKind::Struct,
                variant: Some("article".into()),
                fields: Vec::new(),
            }),
        ],
        expand: Vec::new(),
        search: None,
        as_of: None,
        order_by: vec![strata_model::OrderBy {
            field: "seq".into(),
            descending: true,
        }],
        limit: Some(50),
        cursor: None,
        include_tombstoned: false,
    };
    if let Some(source) = query.get("sourceId").and_then(Value::as_str) {
        spec.r#where = serde_json::from_value(json!({"eq": ["sourceId", source]})).ok();
    }
    if let Some(text) = query
        .get("query")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        spec.order_by.clear();
        spec.has_facets.clear();
        spec.search = Some(strata_model::Search {
            text: Some(text.to_string()),
            vector: None,
            variant: None,
            k: 50,
        });
    }
    let page = state
        .services
        .strata()
        .query(spec)
        .await
        .map_err(ApiError::from)?;
    let changes = state
        .services
        .strata()
        .changes(0, Some(50))
        .await
        .map_err(ApiError::from)?;
    let snapshot = changes.last().map(|change| change.snapshot).unwrap_or(0);
    let mut content_ids = Vec::new();
    let mut frozen = Vec::new();
    for row in page.rows {
        let Some(rid) = row["rid"].as_str() else {
            continue;
        };
        content_ids.push(rid.to_string());
        frozen.push(json!({
            "kind": "content",
            "contentId": rid,
            "title": row.pointer("/struct:article/value/title").cloned().unwrap_or(Value::Null),
            "text": row.pointer("/struct:article/value/summary").cloned().unwrap_or(Value::Null),
        }));
    }
    Ok(json!({
        "kind": "querySnapshot",
        "query": query,
        "snapshot": snapshot,
        "contentIds": content_ids,
        "items": frozen,
    }))
}

async fn freeze_url(state: &AppState, url: &str) -> Result<Value, ApiError> {
    let parsed =
        url::Url::parse(url).map_err(|_| ApiError::validation("url", "format", "地址无效"))?;
    let Some(host) = parsed.host_str() else {
        return Err(ApiError::validation("url", "format", "地址缺少主机"));
    };
    if host == "localhost"
        || host == "127.0.0.1"
        || host == "::1"
        || host.starts_with("10.")
        || host.starts_with("192.168.")
    {
        return Err(ApiError::validation(
            "url",
            "policy",
            "不允许指向本机或内网地址",
        ));
    }
    let reply = workflow::http_request(
        "GET",
        url,
        &[("user-agent".into(), "ModalPrism/1.0".into())],
        None,
        Duration::from_secs(20),
        &state.services.proxy_setting(),
    )
    .await
    .map_err(|error| ApiError::dependency(error.to_string()))?;
    let text: String = reply.body.chars().take(200_000).collect();
    Ok(json!({
        "kind": "url",
        "url": url,
        "status": reply.status,
        "text": text,
        "contentHash": util::hash_short(&reply.body),
    }))
}

async fn freeze_input(state: &AppState, input: &Value) -> Result<Value, ApiError> {
    match input["kind"].as_str().unwrap_or_default() {
        "content" => freeze_content(state, input["contentId"].as_str().unwrap_or_default()).await,
        "contentList" => {
            let ids = input["contentIds"].as_array().cloned().unwrap_or_default();
            let mut items = Vec::new();
            for id in ids {
                if let Some(id) = id.as_str() {
                    items.push(freeze_content(state, id).await?);
                }
            }
            Ok(json!({"kind": "contentList", "items": items}))
        }
        "querySnapshot" => {
            let query = input["query"].as_object().cloned().unwrap_or_default();
            freeze_query(state, &query).await
        }
        "text" => Ok(json!({"kind": "text", "text": input["text"]})),
        "structured" => Ok(json!({"kind": "structured", "value": input["value"]})),
        "url" => freeze_url(state, input["url"].as_str().unwrap_or_default()).await,
        "file" => {
            let file_id = input["fileId"].as_str().unwrap_or_default();
            let doc = state
                .store
                .get_document(UPLOAD, file_id)
                .await?
                .ok_or_else(|| ApiError::not_found(format!("文件 {file_id}")))?;
            Ok(json!({
                "kind": "file",
                "fileId": file_id,
                "name": doc.body["name"],
                "mime": doc.body["mime"],
                "size": doc.body["size"],
                "contentHash": doc.body["contentHash"],
                "text": doc.body["text"],
            }))
        }
        other => Err(ApiError::validation(
            "kind",
            "enum",
            format!("未知输入类型: {other}"),
        )),
    }
}

async fn create_input_set(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["name", "inputs"])?;
    let name = util::non_empty(map, "name")?;
    let inputs = map
        .get("inputs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if inputs.is_empty() || inputs.len() > 50 {
        return Err(ApiError::validation(
            "inputs",
            "outOfRange",
            "数量需要在 1 到 50 之间",
        ));
    }
    let refs = input_refs(&inputs)?;
    let mut frozen = Vec::new();
    for input in &refs {
        frozen.push(freeze_input(&state, input).await?);
    }
    let id = new_id("iset");
    let doc = json!({
        "id": id,
        "name": name,
        "inputs": frozen,
        "createdAt": now(),
    });
    state.store.insert_document(INPUTSET, &id, &doc).await?;
    crate::observe::append_audit(
        &state,
        "production.inputSet.create",
        "succeeded",
        "冻结生产输入",
        json!({"id": id, "count": frozen.len()}),
        None,
    )
    .await?;
    Ok(util::created(&doc))
}

async fn upload_input(
    State(state): State<Shared>,
    mut multipart: Multipart,
) -> Result<Response, ApiError> {
    let mut name = None;
    let mut bytes = Vec::new();
    let mut mime = "application/octet-stream".to_string();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| ApiError::invalid(format!("读取上传失败: {error}")))?
    {
        match field.name() {
            Some("file") => {
                name = field.file_name().map(str::to_string);
                if let Some(kind) = field.content_type() {
                    mime = kind.to_string();
                }
                bytes = field
                    .bytes()
                    .await
                    .map_err(|error| ApiError::invalid(format!("读取文件失败: {error}")))?
                    .to_vec();
            }
            Some("name") => {
                name = Some(
                    field
                        .text()
                        .await
                        .map_err(|error| ApiError::invalid(format!("读取名称失败: {error}")))?,
                );
            }
            _ => {}
        }
    }
    if bytes.is_empty() {
        return Err(ApiError::validation("file", "required", "文件为空"));
    }
    if bytes.len() > 20 * 1024 * 1024 {
        return Err(ApiError::validation("file", "tooLarge", "文件超过 20 MiB"));
    }
    let id = new_id("file");
    let upload_dir = state.data_dir.join("uploads");
    tokio::fs::create_dir_all(&upload_dir)
        .await
        .map_err(|error| ApiError::internal(format!("创建上传目录失败: {error}")))?;
    let path = upload_dir.join(&id);
    tokio::fs::write(&path, &bytes)
        .await
        .map_err(|error| ApiError::internal(format!("保存上传失败: {error}")))?;
    let text = if mime.starts_with("text/")
        || mime.contains("json")
        || mime.contains("xml")
        || mime.contains("yaml")
    {
        Some(
            String::from_utf8_lossy(&bytes)
                .chars()
                .take(200_000)
                .collect::<String>(),
        )
    } else {
        None
    };
    let hash = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let doc = json!({
        "id": id,
        "name": name.unwrap_or_else(|| id.clone()),
        "mime": mime,
        "size": bytes.len(),
        "contentHash": hash,
        "text": text,
        "createdAt": now(),
    });
    state.store.insert_document(UPLOAD, &id, &doc).await?;
    Ok(util::created(&doc))
}

async fn list_workflows(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    list_docs(&state, WORKFLOW, query, None).await
}

async fn create_workflow(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["name", "description"])?;
    let name = util::non_empty(map, "name")?;
    let description = util::text(map, "description")?;
    let id = new_id("wf");
    let doc = json!({
        "id": id,
        "name": name,
        "description": description,
        "latestVersionId": Value::Null,
        "versionCount": 0,
        "etag": format!("workflow-{id}-v1"),
        "createdAt": now(),
    });
    state.store.insert_document(WORKFLOW, &id, &doc).await?;
    crate::observe::append_audit(
        &state,
        "production.workflow.create",
        "succeeded",
        "创建工作流",
        json!({"id": id}),
        None,
    )
    .await?;
    Ok(util::created(&doc))
}

async fn list_versions(
    State(state): State<Shared>,
    Path(workflow_id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    state
        .store
        .get_document(WORKFLOW, &workflow_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("工作流 {workflow_id}")))?;
    let docs = state.store.list_documents(VERSION).await?;
    let items: Vec<Value> = docs
        .into_iter()
        .map(|doc| doc.body)
        .filter(|body| body["workflowId"] == workflow_id)
        .collect();
    let limit = util::limit(query.limit.as_deref())?;
    let filter = format!("workflow-versions:{workflow_id}");
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

fn validate_steps(steps: &[Value]) -> Result<(), ApiError> {
    let mut ids = BTreeSet::new();
    for step in steps {
        let Some(map) = step.as_object() else {
            return Err(ApiError::validation("steps", "type", "步骤必须是对象"));
        };
        util::unknown(
            map,
            &[
                "id",
                "kind",
                "modelId",
                "promptVersionId",
                "skillVersionIds",
                "dependsOn",
                "parameters",
            ],
        )?;
        let id = util::non_empty(map, "id")?;
        if !ids.insert(id.clone()) {
            return Err(ApiError::validation(
                "steps",
                "duplicate",
                format!("步骤 id 重复: {id}"),
            ));
        }
        let kind = util::text(map, "kind")?;
        if !matches!(
            kind.as_str(),
            "model" | "template" | "externalCall" | "validation"
        ) {
            return Err(ApiError::validation("kind", "enum", "未知步骤类型"));
        }
    }
    for step in steps {
        let id = step["id"].as_str().unwrap_or_default();
        for dep in step["dependsOn"].as_array().cloned().unwrap_or_default() {
            let Some(dep) = dep.as_str() else { continue };
            if !ids.contains(dep) {
                return Err(ApiError::validation(
                    "dependsOn",
                    "unknown",
                    format!("{id} 依赖不存在的步骤 {dep}"),
                ));
            }
            if dep == id {
                return Err(ApiError::validation(
                    "dependsOn",
                    "cycle",
                    "步骤不能依赖自身",
                ));
            }
        }
    }
    // 有向无环检测。
    let mut remaining: BTreeSet<String> = ids.clone();
    let mut resolved = BTreeSet::new();
    loop {
        let ready: Vec<String> = remaining
            .iter()
            .filter(|id| {
                steps
                    .iter()
                    .find(|step| step["id"] == id.as_str())
                    .map(|step| {
                        step["dependsOn"]
                            .as_array()
                            .cloned()
                            .unwrap_or_default()
                            .iter()
                            .filter_map(Value::as_str)
                            .all(|dep| resolved.contains(dep))
                    })
                    .unwrap_or(true)
            })
            .cloned()
            .collect();
        if ready.is_empty() {
            if remaining.is_empty() {
                return Ok(());
            }
            return Err(ApiError::validation("steps", "cycle", "步骤依赖存在循环"));
        }
        for id in ready {
            resolved.insert(id.clone());
            remaining.remove(&id);
        }
    }
}

async fn create_version(
    State(state): State<Shared>,
    Path(workflow_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["steps"])?;
    let steps = map
        .get("steps")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| ApiError::validation("steps", "required", "不能为空"))?;
    if steps.is_empty() || steps.len() > 50 {
        return Err(ApiError::validation(
            "steps",
            "outOfRange",
            "步骤数量需要在 1 到 50 之间",
        ));
    }
    validate_steps(&steps)?;
    let workflow = state
        .store
        .get_document(WORKFLOW, &workflow_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("工作流 {workflow_id}")))?;
    util::if_match(
        &headers,
        &format!("workflow-{workflow_id}-v{}", workflow.version),
    )?;
    let number = workflow.body["versionCount"].as_i64().unwrap_or(0) + 1;
    let id = new_id("wfver");
    let doc = json!({
        "id": id,
        "workflowId": workflow_id,
        "version": number,
        "steps": steps,
        "createdAt": now(),
    });
    state.store.insert_document(VERSION, &id, &doc).await?;
    let mut next = workflow.body;
    next["latestVersionId"] = Value::String(id.clone());
    next["versionCount"] = json!(number);
    state
        .store
        .update_document(WORKFLOW, &workflow_id, &next, Some(workflow.version))
        .await?;
    crate::observe::append_audit(
        &state,
        "production.workflow.version",
        "succeeded",
        "发布工作流版本",
        json!({"workflowId": workflow_id, "versionId": id, "version": number}),
        None,
    )
    .await?;
    Ok(util::created(&doc))
}

async fn create_run(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["inputSetId", "workflowVersionId", "mode"])?;
    let input_set_id = util::non_empty(map, "inputSetId")?;
    let version_id = util::non_empty(map, "workflowVersionId")?;
    let mode = util::optional_text(map, "mode")?.unwrap_or_else(|| "manual".into());
    if !matches!(mode.as_str(), "automatic" | "manual") {
        return Err(ApiError::validation(
            "mode",
            "enum",
            "只支持 automatic 或 manual",
        ));
    }
    state
        .store
        .get_document(INPUTSET, &input_set_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("输入集 {input_set_id}")))?;
    let version = state
        .store
        .get_document(VERSION, &version_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("工作流版本 {version_id}")))?;
    let workflow_id = version.body["workflowId"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    // 前置校验：版本里引用了模型步骤而系统未配置任何模型时，建运行即 412。
    let needs_model = version.body["steps"]
        .as_array()
        .map(|steps| {
            steps.iter().any(|step| {
                step["kind"] == json!("model")
                    && step["modelId"].as_str().unwrap_or_default().is_empty()
            })
        })
        .unwrap_or(false);
    if needs_model && crate::settings::default_model(&state).await?.is_none() {
        return Err(crate::search::precondition_requirements(vec![json!({
            "type": "model",
            "setting": "ai.defaultModelId",
            "message": "尚未配置模型：生产/加工路径会拒绝启动",
            "where": "设置 → 模型",
        })]));
    }
    let run_id = new_id("run");
    let task_id = new_id("ctask");
    let run = json!({
        "id": run_id,
        "inputSetId": input_set_id,
        "workflowVersionId": version_id,
        "workflowId": workflow_id,
        "mode": mode,
        "status": "queued",
        "attemptIds": [],
        "candidateIds": [],
        "taskId": task_id,
        "createdAt": now(),
        "updatedAt": now(),
    });
    state.store.insert_document(RUN, &run_id, &run).await?;
    crate::operations::record_task(
        &state,
        json!({
            "id": task_id,
            "kind": "production",
            "title": format!("生产运行 {run_id}"),
            "status": "queued",
            "runId": run_id,
            "createdAt": now(),
            "updatedAt": now(),
        }),
    )
    .await?;
    let operation = crate::operations::start(
        &state,
        "production.run",
        vec![
            json!({"kind": "productionRun", "id": run_id}),
            json!({"kind": "task", "id": task_id}),
        ],
        json!({"kind": "production.run", "runId": run_id}),
    )
    .await?;
    Ok(accepted(&operation))
}

async fn list_runs(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    list_docs(
        &state,
        RUN,
        query.clone(),
        query.status.as_deref().filter(|value| !value.is_empty()),
    )
    .await
}

async fn get_run(
    State(state): State<Shared>,
    Path(run_id): Path<String>,
) -> Result<Response, ApiError> {
    let doc = state
        .store
        .get_document(RUN, &run_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("运行 {run_id}")))?;
    Ok(util::ok(&doc.body))
}

async fn list_attempts(
    State(state): State<Shared>,
    Path(run_id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    state
        .store
        .get_document(RUN, &run_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("运行 {run_id}")))?;
    let docs = state.store.list_documents(ATTEMPT).await?;
    let items: Vec<Value> = docs
        .into_iter()
        .map(|doc| doc.body)
        .filter(|body| body["runId"] == run_id)
        .collect();
    let limit = util::limit(query.limit.as_deref())?;
    let filter = format!("attempts:{run_id}");
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

async fn retry_run(
    State(state): State<Shared>,
    Path(run_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["reason"])?;
    let reason = util::optional_text(map, "reason")?;
    let run = state
        .store
        .get_document(RUN, &run_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("运行 {run_id}")))?;
    if !matches!(
        run.body["status"].as_str().unwrap_or_default(),
        "failed" | "partiallySucceeded" | "cancelled" | "awaitingReview" | "succeeded"
    ) {
        return Err(ApiError::conflict("当前状态不允许重试"));
    }
    let operation = crate::operations::start(
        &state,
        "production.retry",
        vec![json!({"kind": "productionRun", "id": run_id})],
        json!({"kind": "production.retry", "runId": run_id, "reason": reason}),
    )
    .await?;
    Ok(accepted(&operation))
}

async fn list_candidates(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    list_docs(
        &state,
        CANDIDATE,
        query.clone(),
        query.status.as_deref().filter(|value| !value.is_empty()),
    )
    .await
}

async fn list_drafts(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    list_docs(&state, DRAFT, query, None).await
}

async fn get_draft(
    State(state): State<Shared>,
    Path(draft_id): Path<String>,
) -> Result<Response, ApiError> {
    let doc = state
        .store
        .get_document(DRAFT, &draft_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("草稿 {draft_id}")))?;
    let etag = format!("draft-{draft_id}-v{}", doc.version);
    let mut response = util::ok(&doc.body);
    if let Ok(value) = etag.parse() {
        response
            .headers_mut()
            .insert(axum::http::header::ETAG, value);
    }
    Ok(response)
}

/// 把模型资源转换成一次调用的声明。密钥只从环境变量读取，不落库。
fn model_spec(model: &Value) -> Result<workflow::ModelSpec, ApiError> {
    let kind = model["kind"].as_str().unwrap_or("remoteApi");
    let mode = match kind {
        "remoteApi" => workflow::Mode::Api,
        "localService" => workflow::Mode::Local,
        "localCli" => workflow::Mode::Cli,
        other => {
            return Err(ApiError::validation(
                "kind",
                "enum",
                format!("未知模型类型: {other}"),
            ));
        }
    };
    let endpoint = model["endpoint"].as_str().unwrap_or_default().to_string();
    let secret_ref = model["secretRef"]
        .as_str()
        .filter(|value| !value.is_empty());
    if let Some(name) = secret_ref
        && std::env::var(name).is_err()
    {
        return Err(ApiError::dependency(format!("环境变量 {name} 未设置")).retryable());
    }
    let timeout = model["timeoutMs"]
        .as_i64()
        .map(util::duration_text)
        .unwrap_or_else(|| "90s".into());
    let headers: Vec<(String, String)> = model["headers"]
        .as_object()
        .map(|map| {
            map.iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(workflow::ModelSpec {
        id: None,
        kind: None,
        name: model["name"].as_str().map(str::to_string),
        description: None,
        mode,
        endpoint: Some(endpoint.clone()),
        model: model["model"].as_str().map(str::to_string),
        key_env: secret_ref.map(str::to_string),
        headers,
        command: if mode == workflow::Mode::Cli {
            endpoint.split_whitespace().map(str::to_string).collect()
        } else {
            Vec::new()
        },
        system: None,
        timeout: Some(timeout),
        max_tokens: model["maxTokens"].as_u64().map(|value| value as u32),
        temperature: model["temperature"].as_f64().map(|value| value as f32),
    })
}

pub async fn call_model(
    state: &AppState,
    model: &Value,
    prompt: &str,
    operation_id: &str,
) -> Result<String, ApiError> {
    if !model["enabled"].as_bool().unwrap_or(false) {
        return Err(ApiError::dependency("模型已停用").retryable());
    }
    let spec = model_spec(model)?;
    crate::operations::check_cancel(state, operation_id).await?;
    let mut spec = spec;
    // 系统设置是生产侧的统一策略；模型资源提供连接信息，采样参数可被全局设置覆盖。
    if let Some(value) = crate::settings::value(state, "ai.temperature")
        .await?
        .as_f64()
    {
        spec.temperature = Some(value as f32);
    }
    if let Some(value) = crate::settings::value(state, "ai.maxTokens")
        .await?
        .as_u64()
    {
        spec.max_tokens = Some(value.min(u32::MAX as u64) as u32);
    }
    if let Some(value) = crate::settings::value(state, "ai.requestTimeoutMs")
        .await?
        .as_u64()
    {
        spec.timeout = Some(crate::util::duration_text(value as i64));
    }
    let proxy = state.services.proxy_setting();
    let reply = workflow::complete_model(&spec, prompt, &proxy)
        .await
        .map_err(|error| {
            let message = error.to_string();
            if message.contains("环境变量") || message.contains("超时") || message.contains("连接")
            {
                ApiError::dependency(message).retryable()
            } else {
                ApiError::dependency(message)
            }
        })?;
    Ok(reply)
}

/// 从模型回复里提取第一段可解析的 JSON。
pub fn extract_json(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Some(value);
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str::<Value>(&trimmed[start..=end]).ok()
}

async fn prepare_attempt(
    state: &AppState,
    run_id: &str,
    retry_of: Option<&str>,
) -> Result<(Value, Value, Value, Value), ApiError> {
    let run_doc = state
        .store
        .get_document(RUN, run_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("运行 {run_id}")))?;
    let input_set = state
        .store
        .get_document(
            INPUTSET,
            run_doc.body["inputSetId"].as_str().unwrap_or_default(),
        )
        .await?
        .ok_or_else(|| ApiError::not_found("输入集"))?;
    let version = state
        .store
        .get_document(
            VERSION,
            run_doc.body["workflowVersionId"]
                .as_str()
                .unwrap_or_default(),
        )
        .await?
        .ok_or_else(|| ApiError::not_found("工作流版本"))?;
    let number = run_doc.body["attemptIds"]
        .as_array()
        .map(|items| items.len() as i64 + 1)
        .unwrap_or(1);
    let attempt_id = new_id("att");
    let attempt = json!({
        "id": attempt_id,
        "runId": run_id,
        "number": number,
        "status": "running",
        "steps": [],
        "retryOfAttemptId": retry_of,
        "createdAt": now(),
        "finishedAt": Value::Null,
        "error": Value::Null,
    });
    state
        .store
        .insert_document(ATTEMPT, &attempt_id, &attempt)
        .await?;
    let mut run = run_doc.body.clone();
    run["status"] = Value::String("running".into());
    run["updatedAt"] = Value::String(now());
    if let Some(list) = run["attemptIds"].as_array_mut() {
        list.push(Value::String(attempt_id.clone()));
    }
    state
        .store
        .update_document(RUN, run_id, &run, Some(run_doc.version))
        .await?;
    Ok((run, attempt, input_set.body, version.body))
}

pub async fn execute_run(
    state: &AppState,
    operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let run_id = action["runId"].as_str().unwrap_or_default();
    let (run, attempt, input_set, version) = prepare_attempt(state, run_id, None).await?;
    execute_attempt(state, operation_id, run, attempt, input_set, version).await
}

pub async fn execute_retry(
    state: &AppState,
    operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let run_id = action["runId"].as_str().unwrap_or_default();
    let run_doc = state
        .store
        .get_document(RUN, run_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("运行 {run_id}")))?;
    let previous = run_doc.body["attemptIds"]
        .as_array()
        .and_then(|items| items.last())
        .and_then(Value::as_str)
        .map(str::to_string);
    let (run, attempt, input_set, version) =
        prepare_attempt(state, run_id, previous.as_deref()).await?;
    execute_attempt(state, operation_id, run, attempt, input_set, version).await
}

struct EngineOutput {
    steps: Vec<Value>,
    outputs: BTreeMap<String, String>,
    error: Option<String>,
    cancelled: bool,
}

async fn execute_attempt(
    state: &AppState,
    operation_id: &str,
    run: Value,
    attempt: Value,
    input_set: Value,
    version: Value,
) -> Result<Value, ApiError> {
    let run_id = run["id"].as_str().unwrap_or_default().to_string();
    let attempt_id = attempt["id"].as_str().unwrap_or_default().to_string();
    let steps = version["steps"].as_array().cloned().unwrap_or_default();
    let inputs = input_set["inputs"].as_array().cloned().unwrap_or_default();
    let output = execute_steps(state, operation_id, &steps, &inputs).await;
    let mut attempt_body = attempt.clone();
    attempt_body["steps"] = json!(output.steps);
    attempt_body["finishedAt"] = Value::String(now());
    if let Some(error) = &output.error {
        attempt_body["status"] = Value::String("failed".into());
        attempt_body["error"] = Value::String(error.clone());
    } else if output.cancelled {
        attempt_body["status"] = Value::String("cancelled".into());
    } else {
        attempt_body["status"] = Value::String("succeeded".into());
    }
    let attempt_doc = state
        .store
        .get_document(ATTEMPT, &attempt_id)
        .await?
        .ok_or_else(|| ApiError::not_found(&attempt_id))?;
    state
        .store
        .update_document(
            ATTEMPT,
            &attempt_id,
            &attempt_body,
            Some(attempt_doc.version),
        )
        .await?;

    let mut candidate_id = Value::Null;
    let mut run_status = if output.error.is_some() {
        "failed"
    } else {
        "succeeded"
    };
    if output.cancelled {
        run_status = "cancelled";
    } else if !output.outputs.is_empty() {
        let body = output.outputs.values().last().cloned().unwrap_or_default();
        let validation = validate_candidate(&body, &output.outputs);
        let status = if validation.is_empty() {
            "awaitingReview"
        } else {
            "validationFailed"
        };
        let title = inputs
            .first()
            .and_then(|input| input["title"].as_str().or_else(|| input["name"].as_str()))
            .map(str::to_string)
            .unwrap_or_else(|| {
                body.lines()
                    .next()
                    .unwrap_or("候选内容")
                    .chars()
                    .take(60)
                    .collect()
            });
        let candidate = json!({
            "id": new_id("cand"),
            "runId": run_id,
            "attemptId": attempt_id,
            "status": status,
            "title": title,
            "body": body,
            "outputs": output.outputs,
            "validation": {"issues": validation},
            "selectedOutputIds": [],
            "createdAt": now(),
        });
        candidate_id = candidate["id"].clone();
        state
            .store
            .insert_document(
                CANDIDATE,
                candidate["id"].as_str().unwrap_or_default(),
                &candidate,
            )
            .await?;
        if status == "awaitingReview" {
            run_status = "awaitingReview";
        } else {
            run_status = "partiallySucceeded";
        }
    }
    let run_doc = state
        .store
        .get_document(RUN, &run_id)
        .await?
        .ok_or_else(|| ApiError::not_found(&run_id))?;
    let mut next_run = run_doc.body;
    next_run["status"] = Value::String(run_status.into());
    next_run["updatedAt"] = Value::String(now());
    if !candidate_id.is_null()
        && let Some(list) = next_run["candidateIds"].as_array_mut()
    {
        list.push(candidate_id.clone());
    }
    state
        .store
        .update_document(RUN, &run_id, &next_run, Some(run_doc.version))
        .await?;
    crate::operations::record_task(
        state,
        json!({
            "id": next_run["taskId"],
            "kind": "production",
            "title": format!("生产运行 {run_id}"),
            "status": if run_status == "failed" { "failed" } else { "succeeded" },
            "runId": run_id,
            "createdAt": next_run["createdAt"],
            "updatedAt": now(),
            "error": output.error,
        }),
    )
    .await?;
    let refs = crate::observe::Refs {
        operation_id: Some(operation_id.to_string()),
        run_id: Some(run_id.clone()),
        attempt_id: Some(attempt_id.clone()),
        workflow_version_id: Some(version["id"].as_str().unwrap_or_default().to_string()),
        ..crate::observe::Refs::default()
    };
    crate::observe::append_event(
        state,
        "production.run",
        "生产运行结束",
        json!({"runId": run_id, "status": run_status, "candidateId": candidate_id, "error": output.error}),
        refs,
    )
    .await?;
    Ok(json!({
        "partial": output.error.is_some() && !output.outputs.is_empty(),
        "runId": run_id,
        "attemptId": attempt_id,
        "candidateId": candidate_id,
        "status": run_status,
        "steps": output.steps,
        "error": output.error,
    }))
}

/// 进程重启后把中断中的运行与尝试标记为失败，保留已生成内容供审核/重试。
pub async fn recover_interrupted(state: &AppState, action: &Value) {
    let Some(run_id) = action["runId"].as_str() else {
        return;
    };
    if let Ok(Some(doc)) = state.store.get_document(RUN, run_id).await
        && matches!(
            doc.body["status"].as_str().unwrap_or_default(),
            "queued" | "running"
        )
    {
        let mut next = doc.body;
        next["status"] = Value::String("failed".into());
        next["error"] = Value::String("进程重启，生产运行已中断".into());
        next["updatedAt"] = Value::String(now());
        if state
            .store
            .update_document(RUN, run_id, &next, Some(doc.version))
            .await
            .is_ok()
        {
            let _ = crate::operations::record_task(
                state,
                json!({
                    "id": next["taskId"],
                    "kind": "production",
                    "title": format!("生产运行 {run_id}"),
                    "status": "failed",
                    "runId": run_id,
                    "createdAt": next["createdAt"],
                    "updatedAt": now(),
                    "error": "进程重启，生产运行已中断",
                }),
            )
            .await;
        }
    }
    if let Ok(docs) = state.store.list_documents(ATTEMPT).await {
        for doc in docs {
            if doc.body["runId"] != run_id || doc.body["status"] != "running" {
                continue;
            }
            let mut next = doc.body;
            next["status"] = Value::String("failed".into());
            next["error"] = Value::String("进程重启，尝试已中断".into());
            next["finishedAt"] = Value::String(now());
            let _ = state
                .store
                .update_document(ATTEMPT, &doc.id, &next, Some(doc.version))
                .await;
        }
    }
}

fn validate_candidate(body: &str, outputs: &BTreeMap<String, String>) -> Vec<Value> {
    let mut issues = Vec::new();
    if body.trim().is_empty() {
        issues.push(json!({"code": "empty", "message": "候选正文为空"}));
    }
    let lower = outputs
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    for banned in ["authorization: bearer", "api_key=", "secret_key="] {
        if lower.contains(banned) {
            issues.push(json!({"code": "sensitive", "message": "候选疑似包含密钥"}));
        }
    }
    issues
}

async fn execute_steps(
    state: &AppState,
    operation_id: &str,
    steps: &[Value],
    inputs: &[Value],
) -> EngineOutput {
    let mut records = Vec::new();
    let mut outputs: BTreeMap<String, String> = BTreeMap::new();
    let mut failed: BTreeSet<String> = BTreeSet::new();
    let mut blocked: BTreeSet<String> = BTreeSet::new();
    let mut started: BTreeSet<String> = BTreeSet::new();
    let mut first_error = None;
    let mut cancelled = false;
    let variables = json!({
        "input": inputs.first().cloned().unwrap_or(Value::Null),
        "inputs": inputs,
    });

    loop {
        if crate::operations::check_cancel(state, operation_id)
            .await
            .is_err()
        {
            cancelled = true;
            break;
        }
        let ready: Vec<Value> = steps
            .iter()
            .filter(|step| {
                let id = step["id"].as_str().unwrap_or_default();
                !started.contains(id)
            })
            .filter(|step| {
                let deps = step["dependsOn"].as_array().cloned().unwrap_or_default();
                deps.iter().filter_map(Value::as_str).all(|dep| {
                    outputs.contains_key(dep) || failed.contains(dep) || blocked.contains(dep)
                })
            })
            .cloned()
            .collect();
        if ready.is_empty() {
            break;
        }
        for step in ready {
            let id = step["id"].as_str().unwrap_or_default().to_string();
            started.insert(id.clone());
            let deps_blocked = step["dependsOn"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter_map(Value::as_str)
                .any(|dep| failed.contains(dep) || blocked.contains(dep));
            if deps_blocked {
                blocked.insert(id.clone());
                records.push(json!({
                    "id": id,
                    "kind": step["kind"],
                    "status": "skipped",
                    "ms": 0,
                    "error": "依赖步骤失败，本分支停止",
                }));
                continue;
            }
            let started_at = std::time::Instant::now();
            let mut vars = variables.clone();
            vars["steps"] = json!(outputs);
            let outcome = run_step(state, operation_id, &step, &vars, inputs).await;
            match outcome {
                Ok(text) => {
                    outputs.insert(id.clone(), text.clone());
                    records.push(json!({
                        "id": id,
                        "kind": step["kind"],
                        "status": "succeeded",
                        "ms": started_at.elapsed().as_millis(),
                        "output": text.chars().take(2000).collect::<String>(),
                    }));
                }
                Err(error) => {
                    failed.insert(id.clone());
                    if first_error.is_none() {
                        first_error = Some(error.message.clone());
                    }
                    records.push(json!({
                        "id": id,
                        "kind": step["kind"],
                        "status": "failed",
                        "ms": started_at.elapsed().as_millis(),
                        "error": error.message,
                    }));
                }
            }
        }
    }
    for step in steps {
        let id = step["id"].as_str().unwrap_or_default();
        if !started.contains(id) {
            records.push(json!({
                "id": id,
                "kind": step["kind"],
                "status": "pending",
                "ms": 0,
            }));
        }
    }
    EngineOutput {
        steps: records,
        outputs,
        error: if cancelled { None } else { first_error },
        cancelled,
    }
}

async fn run_step(
    state: &AppState,
    operation_id: &str,
    step: &Value,
    vars: &Value,
    _inputs: &[Value],
) -> Result<String, ApiError> {
    let parameters = step["parameters"].as_object().cloned().unwrap_or_default();
    let kind = step["kind"].as_str().unwrap_or_default();
    match kind {
        "model" => {
            let model = if let Some(id) = step["modelId"].as_str().filter(|value| !value.is_empty())
            {
                crate::settings::model(state, id).await?
            } else {
                crate::settings::default_model(state)
                    .await?
                    .ok_or_else(|| ApiError::dependency("未配置默认模型"))?
            };
            let prompt = if let Some(version) = step["promptVersionId"]
                .as_str()
                .filter(|value| !value.is_empty())
            {
                crate::settings::prompt_content(state, version).await?
            } else {
                parameters
                    .get("prompt")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ApiError::validation("parameters.prompt", "required", "模型步骤需要提示词")
                    })?
                    .to_string()
            };
            let mut skills = String::new();
            if let Some(ids) = step["skillVersionIds"].as_array() {
                for id in ids.iter().filter_map(Value::as_str) {
                    skills.push_str(&crate::settings::skill_content(state, id).await?);
                    skills.push('\n');
                }
            }
            let rendered = workflow::render_template(&prompt, vars)
                .map_err(|error| ApiError::validation("prompt", "template", error.to_string()))?;
            let full = if skills.is_empty() {
                rendered
            } else {
                format!("执行约束：\n{skills}\n\n{rendered}")
            };
            call_model(state, &model, &full, operation_id).await
        }
        "template" => {
            let template = parameters
                .get("template")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ApiError::validation("parameters.template", "required", "模板步骤需要 template")
                })?;
            workflow::render_template(template, vars).map_err(|error| {
                ApiError::validation("parameters.template", "template", error.to_string())
            })
        }
        "externalCall" => {
            let url = parameters
                .get("url")
                .and_then(Value::as_str)
                .map(|value| workflow::render_template(value, vars))
                .transpose()
                .map_err(|error| {
                    ApiError::validation("parameters.url", "template", error.to_string())
                })?
                .ok_or_else(|| {
                    ApiError::validation("parameters.url", "required", "外部调用需要 url")
                })?;
            let method = parameters
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("POST")
                .to_string();
            let headers: Vec<(String, String)> = parameters
                .get("headers")
                .and_then(Value::as_object)
                .map(|map| {
                    map.iter()
                        .filter_map(|(key, value)| {
                            value.as_str().map(|value| (key.clone(), value.to_string()))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let body = parameters
                .get("body")
                .and_then(Value::as_str)
                .map(|value| workflow::render_template(value, vars))
                .transpose()
                .map_err(|error| {
                    ApiError::validation("parameters.body", "template", error.to_string())
                })?;
            let reply = workflow::http_request(
                &method,
                &url,
                &headers,
                body,
                Duration::from_secs(30),
                &state.services.proxy_setting(),
            )
            .await
            .map_err(|error| ApiError::dependency(error.to_string()))?;
            reply
                .ok("外部调用")
                .map_err(|error| ApiError::dependency(error.to_string()))
        }
        "validation" => {
            let schema = parameters.get("schema").cloned().ok_or_else(|| {
                ApiError::validation("parameters.schema", "required", "校验步骤需要 schema")
            })?;
            let target = parameters
                .get("value")
                .cloned()
                .or_else(|| {
                    step["dependsOn"]
                        .as_array()
                        .and_then(|items| items.first())
                        .and_then(Value::as_str)
                        .and_then(|id| vars["steps"].get(id))
                        .cloned()
                })
                .unwrap_or_else(|| vars["steps"].clone());
            let parsed = if let Some(text) = target.as_str() {
                serde_json::from_str::<Value>(text)
                    .unwrap_or_else(|_| Value::String(text.to_string()))
            } else {
                target
            };
            let validator = jsonschema::validator_for(&schema).map_err(|error| {
                ApiError::validation("parameters.schema", "invalid", error.to_string())
            })?;
            if !validator.is_valid(&parsed) {
                let errors: Vec<String> = validator
                    .iter_errors(&parsed)
                    .map(|error| error.to_string())
                    .collect();
                return Err(ApiError::validation(
                    "parameters.schema",
                    "mismatch",
                    errors.join("; "),
                ));
            }
            Ok(serde_json::to_string(&parsed).unwrap_or_default())
        }
        other => Err(ApiError::validation(
            "kind",
            "enum",
            format!("未知步骤类型: {other}"),
        )),
    }
}

async fn review_candidate(
    State(state): State<Shared>,
    Path(candidate_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["decision", "comment", "selectedOutputIds"])?;
    let decision = util::text(map, "decision")?;
    if !matches!(
        decision.as_str(),
        "approve" | "reject" | "select" | "returnForRegeneration"
    ) {
        return Err(ApiError::validation("decision", "enum", "未知审核决定"));
    }
    let comment = util::optional_text(map, "comment")?.unwrap_or_default();
    let selected = util::optional_strings(map, "selectedOutputIds")?.unwrap_or_default();
    let candidate = state
        .store
        .get_document(CANDIDATE, &candidate_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("候选 {candidate_id}")))?;
    util::if_match(
        &headers,
        &format!("candidate-{candidate_id}-v{}", candidate.version),
    )?;
    let status = candidate.body["status"].as_str().unwrap_or_default();
    if decision == "approve" && !matches!(status, "awaitingReview" | "approved") {
        return Err(ApiError::conflict("当前候选不可批准"));
    }
    let review_id = new_id("review");
    let review = json!({
        "id": review_id,
        "candidateId": candidate_id,
        "decision": decision,
        "comment": comment,
        "selectedOutputIds": selected,
        "createdAt": now(),
    });
    state
        .store
        .insert_document(REVIEW, &review_id, &review)
        .await?;
    let mut next = candidate.body.clone();
    match decision.as_str() {
        "approve" => next["status"] = Value::String("approved".into()),
        "reject" => next["status"] = Value::String("rejected".into()),
        "select" => {
            next["selectedOutputIds"] = json!(selected);
        }
        "returnForRegeneration" => {
            next["status"] = Value::String("superseded".into());
            let run_id = next["runId"].as_str().unwrap_or_default().to_string();
            let operation = crate::operations::start(
                &state,
                "production.retry",
                vec![json!({"kind": "productionRun", "id": run_id})],
                json!({"kind": "production.retry", "runId": run_id, "reason": "候选退回重生成"}),
            )
            .await?;
            next["regenerationOperationId"] = operation["operationId"].clone();
        }
        _ => {}
    }
    next["updatedAt"] = Value::String(now());
    state
        .store
        .update_document(CANDIDATE, &candidate_id, &next, Some(candidate.version))
        .await?;
    crate::observe::append_audit(
        &state,
        "production.review",
        "succeeded",
        "审核候选",
        json!({"candidateId": candidate_id, "decision": decision}),
        None,
    )
    .await?;
    Ok(util::created(&review))
}

async fn create_draft(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["candidateIds", "title", "body"])?;
    let candidate_ids = util::strings(map, "candidateIds")?;
    let title = util::non_empty(map, "title")?;
    let content = util::non_empty(map, "body")?;
    if candidate_ids.is_empty() {
        return Err(ApiError::validation("candidateIds", "required", "不能为空"));
    }
    for id in &candidate_ids {
        let candidate = state
            .store
            .get_document(CANDIDATE, id)
            .await?
            .ok_or_else(|| ApiError::not_found(format!("候选 {id}")))?;
        if candidate.body["status"] != "approved" {
            return Err(ApiError::conflict(format!("候选 {id} 尚未批准")));
        }
    }
    let id = new_id("draft");
    let version = json!({"version": 1, "title": title, "body": content, "createdAt": now()});
    let doc = json!({
        "id": id,
        "candidateIds": candidate_ids,
        "title": title,
        "body": content,
        "latestVersion": 1,
        "versions": [version],
        "etag": format!("draft-{id}-v1"),
        "createdAt": now(),
        "updatedAt": now(),
    });
    state.store.insert_document(DRAFT, &id, &doc).await?;
    crate::observe::append_audit(
        &state,
        "production.draft.create",
        "succeeded",
        "创建草稿",
        json!({"id": id, "candidates": candidate_ids}),
        None,
    )
    .await?;
    Ok(util::created(&doc))
}

async fn update_draft(
    State(state): State<Shared>,
    Path(draft_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["candidateIds", "title", "body"])?;
    let title = util::non_empty(map, "title")?;
    let content = util::non_empty(map, "body")?;
    let doc = state
        .store
        .get_document(DRAFT, &draft_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("草稿 {draft_id}")))?;
    util::if_match(&headers, &format!("draft-{draft_id}-v{}", doc.version))?;
    let mut next = doc.body.clone();
    let latest = next["latestVersion"].as_i64().unwrap_or(1) + 1;
    next["title"] = Value::String(title.clone());
    next["body"] = Value::String(content.clone());
    next["latestVersion"] = json!(latest);
    next["updatedAt"] = Value::String(now());
    if let Some(versions) = next["versions"].as_array_mut() {
        versions
            .push(json!({"version": latest, "title": title, "body": content, "createdAt": now()}));
    }
    state
        .store
        .update_document(DRAFT, &draft_id, &next, Some(doc.version))
        .await?;
    let mut response = util::ok(&next);
    if let Ok(value) = format!("draft-{draft_id}-v{}", doc.version + 1).parse() {
        response
            .headers_mut()
            .insert(axum::http::header::ETAG, value);
    }
    Ok(response)
}
