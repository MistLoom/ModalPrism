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
    "delivery_rule",
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

/// 确保默认项目存在，并把未标注归属的历史信源一次性划到默认项目并标记共享（幂等）。
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
    // 历史全局信源：凡正文未标注 project 的信源，一次性归默认项目并标为共享，
    // 使其对所有项目可见；已标注的（功能上线后新建）保持不变。
    let entries = config::entries(&state.config_dir, "source").map_err(ApiError::from)?;
    let mut changed = false;
    for entry in entries {
        if entry.doc.get("project").is_some() {
            continue;
        }
        let mut doc = entry.doc;
        doc["project"] = json!(DEFAULT_PROJECT_ID);
        doc["shared"] = json!(true);
        config::update(
            &state.config_dir,
            &crate::catalog::options(),
            &config::Draft {
                kind: "source".into(),
                slug: entry.slug.clone(),
                doc,
            },
        )
        .map_err(ApiError::from)?;
        changed = true;
    }
    // 仅在真的改写过信源正文时重载配置，避免无变更启动也推高配置代数。
    if changed {
        state.services.config_reload().map_err(ApiError::from)?;
    }
    // 默认项目是所有内置信源的归属，内置分类也跟着它落地：启动与恢复出厂都会走到这里。
    crate::source_groups::seed_builtin(state).await?;
    // 旧配置里内置信源仍把节律写在 `schedule` 上：只摘除、不补计划。自带计划只留
    // 少量示例，未被示例覆盖的信源不参与调度，避免「没有计划却在采集」。
    let slugs: Vec<&str> = config::source_type::BUILTIN_SOURCES
        .iter()
        .map(|source| source.slug)
        .collect();
    match config::source_type::strip_builtin_schedules(
        &state.config_dir,
        &crate::catalog::options(),
        &slugs,
    ) {
        Ok(0) => {}
        Ok(stripped) => {
            state.services.config_reload().map_err(ApiError::from)?;
            tracing::info!(stripped, "内置信源遗留的节律已摘除");
        }
        Err(error) => tracing::warn!(detail = %error, "内置信源节律摘除失败"),
    }
    // 旧配置（升级或从旧基线恢复）里内置信源一个归属都没有时，按内置分类补齐。
    crate::source_groups::file_builtin(state).await?;
    // 出厂基线要跟随版本：内置分类/信源清单变化后作废旧基线，
    // `AppState::open` 紧随其后的 ensure_factory_snapshot 会按当前配置重建。
    refresh_factory_baseline(state).await?;
    Ok(())
}

/// 让出厂基线代表当前版本的自带数据。
///
/// 基线曾在首次安装时一次性冻结，后续版本新增的内置信源与分类永远进不去，
/// 恢复出厂就会退回旧配置。指纹未变时什么都不做；变了就删掉旧基线并记下
/// 新指纹，重建由启动流程里随后的 `ensure_factory_snapshot` 完成。
async fn refresh_factory_baseline(state: &AppState) -> Result<(), ApiError> {
    let base = state.data_dir.join("factory");
    let stamp = base.join("revision");
    let revision = config::source_type::builtin_fingerprint().to_string();
    let current = tokio::fs::read_to_string(&stamp)
        .await
        .ok()
        .map(|text| text.trim().to_string());
    if current.as_deref() == Some(revision.as_str()) {
        return Ok(());
    }
    // 现行配置不健康时不动旧基线：重建会失败，留着至少还能恢复出厂。
    if let Err(error) = config::validate(&state.config_dir, &crate::catalog::options()) {
        tracing::warn!(detail = %error, "当前配置未通过校验，暂不更新出厂基线版本");
        return Ok(());
    }
    let root = base.join("config");
    if root.exists() {
        tokio::fs::remove_dir_all(&root)
            .await
            .map_err(|error| ApiError::internal(format!("作废过期出厂基线失败: {error}")))?;
        tracing::info!("出厂基线版本过期，已移除，将按当前配置重建");
    }
    tokio::fs::create_dir_all(&base)
        .await
        .map_err(|error| ApiError::internal(format!("建立出厂基线目录失败: {error}")))?;
    tokio::fs::write(&stamp, format!("{revision}\n"))
        .await
        .map_err(|error| ApiError::internal(format!("记录出厂基线版本失败: {error}")))?;
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
    // 项目下已归属的资源先解绑，避免悬空引用；分类元数据随项目一并清理。
    let bindings = state.store.list_documents(BINDING_KIND).await?;
    for doc in bindings {
        if doc.body["projectId"] == project_id {
            state.store.delete_document(BINDING_KIND, &doc.id).await?;
        }
    }
    crate::source_groups::delete_for_project(&state, &project_id).await?;
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

/// 项目资源清单：信源按归属/共享派生（含其他项目的共享信源），
/// 其余落库资源（如 search_record）经 project_binding 关联。
async fn list_resources(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
    Query(query): Query<ResourceQuery>,
) -> Result<Response, ApiError> {
    let _ = project_body(&state, &project_id).await?;
    let wanted = query.kind.as_deref().filter(|value| !value.is_empty());
    let bindings = state.store.list_documents(BINDING_KIND).await?;
    let mut items = Vec::new();
    for doc in &bindings {
        let kind = doc.body["resourceKind"].as_str().unwrap_or_default();
        // 信源归属不再看 binding（改由正文 project/shared 派生）。
        if kind == "source" {
            continue;
        }
        if doc.body["projectId"] != project_id {
            continue;
        }
        if let Some(filter) = wanted {
            if kind != filter {
                continue;
            }
        }
        items.push(json!({
            "kind": kind,
            "id": doc.body["resourceId"].as_str().unwrap_or_default(),
            "boundAt": doc.body["boundAt"],
        }));
    }
    if wanted.is_none() || wanted == Some("source") {
        let sources = crate::catalog::source_rows(&state).await?;
        for source in sources {
            if !crate::catalog::source_visible(&source, &project_id) {
                continue;
            }
            items.push(json!({
                "kind": "source",
                "id": source["id"].as_str().unwrap_or_default(),
                "shared": source["shared"].as_bool().unwrap_or(false),
            }));
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
