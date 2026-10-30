//! 信源分类：用户自定义的管理分组。
//!
//! 分类与 `sourceType` 正交：类型由信源形状决定，是闭集；分类由用户命名，用于
//! 「把信源按自己的口径管起来」。分类归属项目，成员关系写在信源正文的 `groupId`
//! 上（配置即代码，热加载后仍然成立）；分类自身只是元数据文档。
//!
//! 仓库自带一批内置分类（见 `config::source_type::BUILTIN_GROUPS`），启动时种子化到
//! 默认项目，让 39 条内置信源开箱就分好类；内置分类可改名、可移出成员，但不可删除。
//!
//! 控制台把选中分类当作工作范围（长时间在该分类内管理），与临时筛选相互独立。

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::util;
use crate::{AppState, Shared, new_id, now};

pub const GROUP_KIND: &str = "source_group";
/// 分类名上限：避免把一段描述塞进导航栏。
const NAME_LIMIT: usize = 40;

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ListQuery {
    /// `all` 取消项目过滤（管理与迁移用）；缺省只列当前项目。
    #[serde(default)]
    scope: Option<String>,
}

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/catalog/sourceGroups", get(list_groups).post(create_group))
        .route("/catalog/sourceGroups/bindings", post(assign_sources))
        .route(
            "/catalog/sourceGroups/{groupId}",
            axum::routing::patch(update_group).delete(delete_group),
        )
}

fn group_name(map: &serde_json::Map<String, Value>) -> Result<String, ApiError> {
    let name = util::non_empty(map, "name")?.trim().to_string();
    if name.chars().count() > NAME_LIMIT {
        return Err(ApiError::validation(
            "name",
            "tooLong",
            format!("分类名最多 {NAME_LIMIT} 个字"),
        ));
    }
    Ok(name)
}

async fn ensure_unique(
    state: &AppState,
    project_id: &str,
    name: &str,
    keep: Option<&str>,
) -> Result<(), ApiError> {
    for doc in state.store.list_documents(GROUP_KIND).await? {
        if Some(doc.id.as_str()) == keep {
            continue;
        }
        if doc.body["project"].as_str() != Some(project_id) {
            continue;
        }
        let existing = doc.body["name"].as_str().unwrap_or_default();
        if existing.eq_ignore_ascii_case(name) {
            return Err(ApiError::conflict(format!("分类「{name}」已存在")));
        }
    }
    Ok(())
}

/// 分类必须存在且归属给定项目；信源只能归入自己项目下的分类。
pub(crate) async fn ensure_group(
    state: &AppState,
    group_id: &str,
    project_id: &str,
) -> Result<(), ApiError> {
    let doc = state
        .store
        .get_document(GROUP_KIND, group_id)
        .await?
        .ok_or_else(|| ApiError::validation("groupId", "notFound", "分类不存在"))?;
    if doc.body["project"].as_str() != Some(project_id) {
        return Err(ApiError::validation(
            "groupId",
            "project",
            "分类归属其他项目，不能把信源移入",
        ));
    }
    Ok(())
}

/// 项目删除时一并清掉它的分类，避免悬空元数据。
pub(crate) async fn delete_for_project(state: &AppState, project_id: &str) -> Result<(), ApiError> {
    for doc in state.store.list_documents(GROUP_KIND).await? {
        if doc.body["project"].as_str() == Some(project_id) {
            state.store.delete_document(GROUP_KIND, &doc.id).await?;
        }
    }
    Ok(())
}

/// 内置信源分类随启动种子化：默认项目里没有就补一份，已有则原样保留
/// （改名与描述都是用户自己的决定，重启不覆盖）。
pub async fn seed_builtin(state: &AppState) -> Result<(), ApiError> {
    for (index, group) in config::source_type::BUILTIN_GROUPS.iter().enumerate() {
        if state
            .store
            .get_document(GROUP_KIND, group.id)
            .await?
            .is_some()
        {
            continue;
        }
        let doc = json!({
            "id": group.id,
            "name": group.name,
            "description": group.description,
            "project": crate::projects::DEFAULT_PROJECT_ID,
            "builtin": true,
            "order": index,
            "createdAt": now(),
            "updatedAt": now(),
        });
        state
            .store
            .insert_document(GROUP_KIND, group.id, &doc)
            .await?;
    }
    Ok(())
}

/// 给内置信源补上默认分类：整份配置里一个内置归属都没有时视为旧配置（升级或从
/// 旧基线恢复），按清单一次性补全；只要已有任一归属就说明用户已在管理，不再插手。
///
/// 这样“自带信源默认有分类”总能成立，而用户把个别信源改到自建分类或移出后
/// 也不会被重启打回。
pub async fn file_builtin(state: &AppState) -> Result<usize, ApiError> {
    let entries = config::entries(&state.config_dir, "source").map_err(ApiError::from)?;
    let builtin: Vec<&config::Entry> = entries
        .iter()
        .filter(|entry| config::source_type::builtin_source_group(&entry.slug).is_some())
        .collect();
    if builtin.is_empty() {
        return Ok(0);
    }
    let already = builtin
        .iter()
        .filter(|entry| entry.doc.get("groupId").and_then(Value::as_str).is_some())
        .count();
    if already > 0 {
        return Ok(0);
    }
    let mut writes = Vec::new();
    for entry in builtin {
        let Some(group_id) = config::source_type::builtin_source_group(&entry.slug) else {
            continue;
        };
        let mut doc = entry.doc.clone();
        doc["groupId"] = json!(group_id);
        writes.push(config::AtomicWrite::write(
            entry.file.clone(),
            config::to_yaml(&doc),
        ));
    }
    let filled = writes.len();
    config::apply_atomic(&state.config_dir, &crate::catalog::options(), &writes)
        .map_err(ApiError::from)?;
    state.services.config_reload()?;
    tracing::info!(filled, "内置信源已按内置分类补齐默认归属");
    Ok(filled)
}

/// 按信源正文的 `groupId` 统计分类成员数；分类所在项目的信源才算成员。
async fn member_counts(
    state: &AppState,
) -> Result<std::collections::BTreeMap<String, usize>, ApiError> {
    let mut counts = std::collections::BTreeMap::new();
    for entry in config::entries(&state.config_dir, "source").map_err(ApiError::from)? {
        let Some(group_id) = entry.doc.get("groupId").and_then(Value::as_str) else {
            continue;
        };
        *counts.entry(group_id.to_string()).or_insert(0) += 1;
    }
    Ok(counts)
}

async fn list_groups(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let scope_all = query.scope.as_deref() == Some("all");
    let active = project.id().to_string();
    let counts = member_counts(&state).await?;
    let mut items: Vec<Value> = state
        .store
        .list_documents(GROUP_KIND)
        .await?
        .into_iter()
        .filter(|doc| scope_all || doc.body["project"].as_str() == Some(active.as_str()))
        .map(|doc| {
            let mut body = doc.body;
            let count = body["id"]
                .as_str()
                .and_then(|id| counts.get(id))
                .copied()
                .unwrap_or(0);
            body["count"] = json!(count);
            body
        })
        .collect();
    // 内置分类固定在前（`order`），用户自建分类按创建时间追加。
    items.sort_by(|a, b| {
        let order = |value: &Value| value["order"].as_u64().unwrap_or(u64::MAX);
        order(a)
            .cmp(&order(b))
            .then_with(|| a["createdAt"].as_str().cmp(&b["createdAt"].as_str()))
    });
    Ok(util::ok(&json!({ "items": items })))
}

async fn create_group(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["name", "description"])?;
    let name = group_name(map)?;
    ensure_unique(&state, project.id(), &name, None).await?;
    let id = new_id("grp");
    let doc = json!({
        "id": id,
        "name": name,
        "description": util::optional_text(map, "description")?.unwrap_or_default(),
        "project": project.id(),
        "createdAt": now(),
        "updatedAt": now(),
    });
    state.store.insert_document(GROUP_KIND, &id, &doc).await?;
    crate::observe::append_audit(
        &state,
        "catalog.sourceGroup.create",
        "succeeded",
        "新建信源分类",
        json!({"groupId": id, "name": name}),
        None,
    )
    .await?;
    Ok((axum::http::StatusCode::CREATED, axum::Json(doc)).into_response())
}

async fn group_body(state: &AppState, group_id: &str) -> Result<(Value, i64), ApiError> {
    let doc = state
        .store
        .get_document(GROUP_KIND, group_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("分类 {group_id}")))?;
    Ok((doc.body, doc.version))
}

async fn update_group(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Path(group_id): Path<String>,
    Json(patch): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&patch)?;
    util::unknown(map, &["name", "description"])?;
    let (mut body, version) = group_body(&state, &group_id).await?;
    if body["project"].as_str() != Some(project.id()) {
        return Err(ApiError::not_found(format!("分类 {group_id}")));
    }
    if map.contains_key("name") {
        let name = group_name(map)?;
        ensure_unique(&state, project.id(), &name, Some(&group_id)).await?;
        body["name"] = json!(name);
    }
    if let Some(description) = map.get("description") {
        body["description"] = json!(description.as_str().unwrap_or_default().trim());
    }
    body["updatedAt"] = json!(now());
    let doc = state
        .store
        .update_document(GROUP_KIND, &group_id, &body, Some(version))
        .await?;
    crate::observe::append_audit(
        &state,
        "catalog.sourceGroup.update",
        "succeeded",
        "重命名信源分类",
        json!({"groupId": group_id, "name": doc.body["name"]}),
        None,
    )
    .await?;
    Ok(util::ok(&doc.body))
}

async fn delete_group(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Path(group_id): Path<String>,
) -> Result<Response, ApiError> {
    let (body, _) = group_body(&state, &group_id).await?;
    if body["project"].as_str() != Some(project.id()) {
        return Err(ApiError::not_found(format!("分类 {group_id}")));
    }
    // 内置分类是开箱骨架：删了会让内置信源落到未分组，重启又会重建，直接拒绝。
    if body["builtin"].as_bool().unwrap_or(false) {
        return Err(ApiError::conflict("内置分类不可删除，可改名或把成员移出"));
    }
    let released = release_members(&state, &group_id).await?;
    state.store.delete_document(GROUP_KIND, &group_id).await?;
    crate::observe::append_audit(
        &state,
        "catalog.sourceGroup.delete",
        "succeeded",
        "删除信源分类",
        json!({"groupId": group_id, "name": body["name"], "released": released}),
        None,
    )
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT.into_response())
}

/// 把该分类下的信源退回「未分组」；一次原子写入，避免半套成员关系。
async fn release_members(state: &AppState, group_id: &str) -> Result<usize, ApiError> {
    let entries = config::entries(&state.config_dir, "source").map_err(ApiError::from)?;
    let mut writes = Vec::new();
    for entry in entries {
        if entry.doc.get("groupId").and_then(Value::as_str) != Some(group_id) {
            continue;
        }
        let mut doc = entry.doc;
        if let Some(map) = doc.as_object_mut() {
            map.remove("groupId");
        }
        writes.push(config::AtomicWrite::write(
            entry.file,
            config::to_yaml(&doc),
        ));
    }
    if writes.is_empty() {
        return Ok(0);
    }
    let moved = writes.len();
    config::apply_atomic(&state.config_dir, &crate::catalog::options(), &writes)
        .map_err(ApiError::from)?;
    state.services.config_reload()?;
    Ok(moved)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssignBody {
    /// 目标分类；`null` 表示退回未分组。
    #[serde(default)]
    group_id: Option<String>,
    source_ids: Vec<String>,
}

/// 批量移动信源：一次请求处理选中的全部成员，成功项按原子写入落盘。
async fn assign_sources(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["groupId", "sourceIds"])?;
    let input: AssignBody = serde_json::from_value(body)
        .map_err(|_| ApiError::validation("sourceIds", "required", "sourceIds 必须是数组"))?;
    if input.source_ids.is_empty() {
        return Err(ApiError::validation(
            "sourceIds",
            "required",
            "至少选择一个信源",
        ));
    }
    let target = input
        .group_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if let Some(group_id) = &target {
        ensure_group(&state, group_id, project.id()).await?;
    }
    let entries = config::entries(&state.config_dir, "source").map_err(ApiError::from)?;
    let mut writes = Vec::new();
    let mut updated = Vec::new();
    let mut skipped = Vec::new();
    for id in &input.source_ids {
        let Some(entry) = entries.iter().find(|entry| &entry.slug == id) else {
            skipped.push(json!({"id": id, "reason": "信源不存在"}));
            continue;
        };
        // 共享信源只读：成员关系属于信源归属项目。
        let owner = entry
            .doc
            .get("project")
            .and_then(Value::as_str)
            .unwrap_or(crate::projects::DEFAULT_PROJECT_ID);
        if owner != project.id() {
            skipped.push(json!({"id": id, "reason": "他项目共享的信源只读"}));
            continue;
        }
        let mut doc = entry.doc.clone();
        match &target {
            Some(group_id) => doc["groupId"] = json!(group_id),
            None => {
                if let Some(map) = doc.as_object_mut() {
                    map.remove("groupId");
                }
            }
        }
        writes.push(config::AtomicWrite::write(
            entry.file.clone(),
            config::to_yaml(&doc),
        ));
        updated.push(id.clone());
    }
    if !writes.is_empty() {
        config::apply_atomic(&state.config_dir, &crate::catalog::options(), &writes)
            .map_err(ApiError::from)?;
        state.services.config_reload()?;
    }
    crate::observe::append_audit(
        &state,
        "catalog.source.group.assign",
        "succeeded",
        if target.is_some() {
            "把信源移入分类"
        } else {
            "把信源移出分类"
        },
        json!({
            "groupId": target,
            "updated": updated.len(),
            "skipped": skipped.len(),
        }),
        None,
    )
    .await?;
    Ok(util::ok(&json!({
        "updated": updated,
        "skipped": skipped,
    })))
}
