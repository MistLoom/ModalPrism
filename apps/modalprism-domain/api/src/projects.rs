//! 项目中心资源模型。
//!
//! `Project` 是元数据实体；信源 / 任务 / 内容 / 工作流版本 / 发布物通过
//! `project_binding` 文档挂到项目下。旧全局数据不迁移物理存储：启动时把
//! 未归属的信源挂在默认项目下展示，符合「默认项目承接旧数据」。

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::util;
use crate::{AppState, Shared, new_id, now};

pub const PROJECT_KIND: &str = "project";
pub const BINDING_KIND: &str = "project_binding";
pub const DEFAULT_PROJECT_ID: &str = "proj_default";

/// 可归属到项目的资源类型。
pub const BINDABLE_KINDS: &[&str] = &[
    "source",
    "task",
    "content",
    "workflow_version",
    "publication",
];

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/projects", get(list_projects).post(create_project))
        .route(
            "/projects/{projectId}",
            get(get_project)
                .patch(update_project)
                .delete(delete_project),
        )
        .route(
            "/projects/{projectId}/resources",
            get(list_resources).post(bind_resource),
        )
        .route(
            "/projects/{projectId}/resources/{kind}/{resourceId}",
            axum::routing::delete(unbind_resource),
        )
}

/// 确保默认项目存在，并把未归属的信源挂到默认项目（一次性，幂等）。
pub async fn seed(state: &AppState) -> Result<(), ApiError> {
    if state
        .store
        .get_document(PROJECT_KIND, DEFAULT_PROJECT_ID)
        .await?
        .is_none()
    {
        let body = json!({
            "id": DEFAULT_PROJECT_ID,
            "name": "默认项目",
            "description": "系统初始化项目，承接历史全局数据。",
            "isDefault": true,
            "createdAt": now(),
            "updatedAt": now(),
        });
        state
            .store
            .insert_document(PROJECT_KIND, DEFAULT_PROJECT_ID, &body)
            .await?;
    }
    // 旧全局信源：凡未出现在任何 binding 里的信源，一次性挂到默认项目。
    let sources = crate::catalog::source_rows(state).await?;
    let bindings = state.store.list_documents(BINDING_KIND).await?;
    let bound: std::collections::HashSet<String> = bindings
        .iter()
        .filter(|doc| doc.body["resourceKind"] == "source")
        .filter_map(|doc| {
            doc.body["resourceId"]
                .as_str()
                .map(|value| value.to_string())
        })
        .collect();
    for source in sources {
        let slug = source["id"].as_str().unwrap_or_default().to_string();
        if slug.is_empty() || bound.contains(&slug) {
            continue;
        }
        bind(state, DEFAULT_PROJECT_ID, "source", &slug).await?;
    }
    Ok(())
}

async fn bind(
    state: &AppState,
    project_id: &str,
    kind: &str,
    resource_id: &str,
) -> Result<(), ApiError> {
    let binding_id = format!("{kind}:{resource_id}");
    let body = json!({
        "projectId": project_id,
        "resourceKind": kind,
        "resourceId": resource_id,
        "boundAt": now(),
    });
    match state.store.get_document(BINDING_KIND, &binding_id).await? {
        Some(doc) => {
            if doc.body["projectId"] != project_id {
                state
                    .store
                    .update_document(BINDING_KIND, &binding_id, &body, Some(doc.version))
                    .await?;
            }
        }
        None => {
            state
                .store
                .insert_document(BINDING_KIND, &binding_id, &body)
                .await?;
        }
    }
    Ok(())
}

async fn project_body(state: &AppState, project_id: &str) -> Result<(Value, i64), ApiError> {
    let doc = state
        .store
        .get_document(PROJECT_KIND, project_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("项目 {project_id}")))?;
    Ok((doc.body, doc.version))
}

/// 供其他模块读取项目实体（向导完成条件评估等）。
pub(crate) async fn project_view(
    state: &AppState,
    project_id: &str,
) -> Result<(Value, i64), ApiError> {
    project_body(state, project_id).await
}

async fn list_projects(State(state): State<Shared>) -> Result<Response, ApiError> {
    let mut items: Vec<Value> = state
        .store
        .list_documents(PROJECT_KIND)
        .await?
        .into_iter()
        .map(|doc| doc.body)
        .collect();
    items.sort_by(|a, b| a["createdAt"].as_str().cmp(&b["createdAt"].as_str()));
    Ok(util::ok(&json!({ "items": items })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateProject {
    name: String,
    #[serde(default)]
    description: Option<String>,
}

async fn create_project(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let input: CreateProject = serde_json::from_value(body)
        .map_err(|_| ApiError::validation("name", "required", "项目名不能为空"))?;
    let name = input.name.trim();
    if name.is_empty() {
        return Err(ApiError::validation("name", "required", "项目名不能为空"));
    }
    let id = new_id("proj");
    let doc = json!({
        "id": id,
        "name": name,
        "description": input.description.unwrap_or_default(),
        "isDefault": false,
        "createdAt": now(),
        "updatedAt": now(),
    });
    state.store.insert_document(PROJECT_KIND, &id, &doc).await?;
    crate::observe::append_audit(
        &state,
        "project.create",
        "succeeded",
        &format!("创建项目 {name}"),
        json!({"projectId": id}),
        None,
    )
    .await?;
    Ok((axum::http::StatusCode::CREATED, axum::Json(doc)).into_response())
}

async fn get_project(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
) -> Result<Response, ApiError> {
    let (body, version) = project_body(&state, &project_id).await?;
    let mut response = util::ok(&body);
    if let Ok(value) = format!("\"g{version}\"").parse() {
        response
            .headers_mut()
            .insert(axum::http::header::ETAG, value);
    }
    Ok(response)
}

async fn update_project(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
    Json(patch): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&patch)?;
    util::unknown(map, &["name", "description"])?;
    let (mut body, version) = project_body(&state, &project_id).await?;
    if let Some(name) = map.get("name") {
        let name = name.as_str().unwrap_or_default().trim();
        if name.is_empty() {
            return Err(ApiError::validation("name", "required", "项目名不能为空"));
        }
        body["name"] = json!(name);
    }
    if let Some(description) = map.get("description") {
        body["description"] = description.clone();
    }
    body["updatedAt"] = json!(now());
    let doc = state
        .store
        .update_document(PROJECT_KIND, &project_id, &body, Some(version))
        .await?;
    Ok(util::ok(&doc.body))
}

async fn delete_project(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
) -> Result<Response, ApiError> {
    let (body, _) = project_body(&state, &project_id).await?;
    if body["isDefault"].as_bool().unwrap_or(false) {
        return Err(ApiError::conflict("默认项目不可删除"));
    }
    // 项目下已归属的资源先解绑，避免悬空引用。
    let bindings = state.store.list_documents(BINDING_KIND).await?;
    for doc in bindings {
        if doc.body["projectId"] == project_id {
            state.store.delete_document(BINDING_KIND, &doc.id).await?;
        }
    }
    state
        .store
        .delete_document(PROJECT_KIND, &project_id)
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT.into_response())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResourceQuery {
    #[serde(default)]
    kind: Option<String>,
}

/// 项目资源清单：已归属资源 ∪（默认项目视角下的未归属旧数据）。
async fn list_resources(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
    Query(query): Query<ResourceQuery>,
) -> Result<Response, ApiError> {
    let (project, _) = project_body(&state, &project_id).await?;
    let is_default = project["isDefault"].as_bool().unwrap_or(false);
    let bindings = state.store.list_documents(BINDING_KIND).await?;
    let mut items = Vec::new();
    let mut bound_sources: std::collections::HashSet<String> = Default::default();
    for doc in &bindings {
        let kind = doc.body["resourceKind"].as_str().unwrap_or_default();
        let resource_id = doc.body["resourceId"].as_str().unwrap_or_default();
        if kind == "source" {
            bound_sources.insert(resource_id.to_string());
        }
        if doc.body["projectId"] != project_id {
            continue;
        }
        if let Some(filter) = query.kind.as_deref().filter(|value| !value.is_empty()) {
            if kind != filter {
                continue;
            }
        }
        items.push(json!({
            "kind": kind,
            "id": resource_id,
            "boundAt": doc.body["boundAt"],
        }));
    }
    // 默认项目承接旧数据：未归属的信源以默认项目视角展示。
    if is_default
        && query
            .kind
            .as_deref()
            .map(|k| k == "source" || k.is_empty())
            .unwrap_or(true)
    {
        let sources = crate::catalog::source_rows(&state).await?;
        for source in sources {
            let slug = source["id"].as_str().unwrap_or_default();
            if !slug.is_empty() && !bound_sources.contains(slug) {
                items.push(json!({
                    "kind": "source",
                    "id": slug,
                    "legacy": true,
                }));
            }
        }
    }
    items.sort_by(|a, b| {
        let ka = a["kind"].as_str().unwrap_or_default();
        let kb = b["kind"].as_str().unwrap_or_default();
        ka.cmp(kb).then_with(|| {
            a["id"]
                .as_str()
                .unwrap_or_default()
                .cmp(b["id"].as_str().unwrap_or_default())
        })
    });
    Ok(util::ok(&json!({ "items": items })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BindBody {
    kind: String,
    id: String,
}

async fn bind_resource(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
    Json(body): Json<BindBody>,
) -> Result<Response, ApiError> {
    if !BINDABLE_KINDS.contains(&body.kind.as_str()) {
        return Err(ApiError::validation(
            "kind",
            "invalid",
            format!("可归属类型: {}", BINDABLE_KINDS.join(", ")),
        ));
    }
    project_body(&state, &project_id).await?;
    bind(&state, &project_id, &body.kind, &body.id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT.into_response())
}

async fn unbind_resource(
    State(state): State<Shared>,
    Path((project_id, kind, resource_id)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    let (project, _) = project_body(&state, &project_id).await?;
    let binding_id = format!("{kind}:{resource_id}");
    match state.store.get_document(BINDING_KIND, &binding_id).await? {
        Some(doc) if doc.body["projectId"] == project_id => {
            state
                .store
                .delete_document(BINDING_KIND, &binding_id)
                .await?;
            Ok(axum::http::StatusCode::NO_CONTENT.into_response())
        }
        Some(_) => Err(ApiError::conflict("资源归属于其他项目")),
        None => {
            // 旧数据隐式归属默认项目，解绑等价于保持现状。
            if project["isDefault"].as_bool().unwrap_or(false) {
                Ok(axum::http::StatusCode::NO_CONTENT.into_response())
            } else {
                Err(ApiError::not_found(format!("绑定 {binding_id}")))
            }
        }
    }
}
