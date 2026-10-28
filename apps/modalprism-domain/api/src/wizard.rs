//! 建项目向导：可恢复的步骤状态机（草稿存项目下），完成条件机器可校验。
//!
//! 主线：建项目 → 场景模板 → 配信源 → 校验采集 → 模型/依赖门闩 → 加工产出
//! → 对外调用说明。每步可 check / complete / skip；整体可 dismiss 直接进控制台。

use axum::Router;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::util;
use crate::{AppState, Shared, now};

const WIZARD_KIND: &str = "wizard";

/// 环节模板（场景级）。`requires` 声明机器校验的完成条件。
struct StepTemplate {
    id: &'static str,
    title: &'static str,
    description: &'static str,
    check: CheckKind,
}

#[derive(Clone, Copy, PartialEq)]
enum CheckKind {
    /// 无自动条件，手动完成或跳过。
    Manual,
    /// 项目下至少归属一个信源。
    HasSource,
    /// 至少一个信源健康状态非初始（采集已验证）。
    CollectionVerified,
    /// 系统设置里已配置至少一个模型。
    HasModel,
    /// 项目下已有加工产出（工作流版本或发布物）。
    HasOutput,
}

const STEPS: &[StepTemplate] = &[
    StepTemplate {
        id: "create_project",
        title: "建项目",
        description: "项目已创建（进入向导即满足）。",
        check: CheckKind::Manual,
    },
    StepTemplate {
        id: "scenario_template",
        title: "选择场景模板",
        description: "从环节模板中选择一条主线，或跳过自建。",
        check: CheckKind::Manual,
    },
    StepTemplate {
        id: "configure_sources",
        title: "配置信源",
        description: "为本项目归属至少一个信源（含搜索引擎信源）。",
        check: CheckKind::HasSource,
    },
    StepTemplate {
        id: "verify_collection",
        title: "校验采集",
        description: "任一信源完成一次健康采集（可选）。",
        check: CheckKind::CollectionVerified,
    },
    StepTemplate {
        id: "model_gate",
        title: "模型与依赖门闩",
        description: "系统设置中配置模型；加工引用模型版本。",
        check: CheckKind::HasModel,
    },
    StepTemplate {
        id: "processing_output",
        title: "加工与产出",
        description: "为项目配置工作流版本或产生发布物。",
        check: CheckKind::HasOutput,
    },
    StepTemplate {
        id: "api_surface",
        title: "对外调用说明",
        description: "确认对外 HTTP/CLI/MCP 表面与读写边界（手动确认）。",
        check: CheckKind::Manual,
    },
];

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/wizard/templates", get(list_templates))
        .route(
            "/projects/{projectId}/wizard",
            get(get_wizard).post(create_wizard),
        )
        .route("/projects/{projectId}/wizard/dismiss", post(dismiss_wizard))
        .route(
            "/projects/{projectId}/wizard/steps/{stepId}/check",
            post(check_step),
        )
        .route(
            "/projects/{projectId}/wizard/steps/{stepId}/complete",
            post(complete_step),
        )
        .route(
            "/projects/{projectId}/wizard/steps/{stepId}/skip",
            post(skip_step),
        )
}

fn template_json() -> Vec<Value> {
    vec![json!({
        "id": "standard",
        "name": "标准采集→加工主线",
        "description": "建项目 → 场景模板 → 配信源 → 校验采集 → 模型门闩 → 加工产出 → 对外调用。",
        "steps": STEPS.iter().map(|step| json!({
            "id": step.id,
            "title": step.title,
            "description": step.description,
            "auto": step.check != CheckKind::Manual,
        })).collect::<Vec<_>>(),
    })]
}

async fn list_templates() -> Result<Response, ApiError> {
    Ok(util::ok(&json!({ "items": template_json() })))
}

fn fresh_steps() -> Vec<Value> {
    STEPS
        .iter()
        .map(|step| {
            json!({
                "id": step.id,
                "status": "pending",
            })
        })
        .collect()
}

async fn wizard_doc(
    state: &AppState,
    project_id: &str,
) -> Result<Option<serde_json::Value>, ApiError> {
    Ok(state
        .store
        .get_document(WIZARD_KIND, project_id)
        .await?
        .map(|doc| doc.body))
}

async fn save_wizard(
    state: &AppState,
    project_id: &str,
    mut body: Value,
) -> Result<Value, ApiError> {
    body["updatedAt"] = json!(now());
    match state.store.get_document(WIZARD_KIND, project_id).await? {
        Some(doc) => {
            let doc = state
                .store
                .update_document(WIZARD_KIND, project_id, &body, Some(doc.version))
                .await?;
            Ok(doc.body)
        }
        None => {
            state
                .store
                .insert_document(WIZARD_KIND, project_id, &body)
                .await?;
            Ok(body)
        }
    }
}

async fn get_wizard(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
) -> Result<Response, ApiError> {
    if let Some(body) = wizard_doc(&state, &project_id).await? {
        return Ok(util::ok(&body));
    }
    Ok(util::ok(&json!({
        "projectId": project_id,
        "status": "not_started",
        "steps": fresh_steps(),
    })))
}

async fn create_wizard(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
) -> Result<Response, ApiError> {
    let body = json!({
        "projectId": project_id,
        "templateId": "standard",
        "status": "active",
        "steps": fresh_steps(),
        "createdAt": now(),
    });
    let saved = save_wizard(&state, &project_id, body).await?;
    // 第一步「建项目」立即视为完成。
    let saved = mark_step(&saved, "create_project", "done");
    let saved = save_wizard(&state, &project_id, saved).await?;
    Ok((axum::http::StatusCode::CREATED, util::ok(&saved)).into_response())
}

fn step_position(step_id: &str) -> Result<usize, ApiError> {
    STEPS
        .iter()
        .position(|step| step.id == step_id)
        .ok_or_else(|| ApiError::not_found(format!("向导环节 {step_id}")))
}

/// 顺序门闩：仅当此前的步骤均已 done|skipped 才允许完成/跳过当前步。
fn ensure_order(wizard: &Value, step_id: &str) -> Result<(), ApiError> {
    let pos = step_position(step_id)?;
    let steps = wizard["steps"].as_array().cloned().unwrap_or_default();
    let pending: Vec<String> = steps
        .iter()
        .take(pos)
        .filter_map(|step| {
            let status = step["status"].as_str().unwrap_or("pending");
            (status != "done" && status != "skipped")
                .then(|| step["id"].as_str().unwrap_or_default().to_string())
        })
        .collect();
    if pending.is_empty() {
        return Ok(());
    }
    Err(ApiError::precondition(format!(
        "前置环节未完成：{}；请先按顺序处理前面的向导步骤",
        pending.join("、")
    )))
}

fn mark_step(wizard: &Value, step_id: &str, status: &str) -> Value {
    let mut wizard = wizard.clone();
    if let Some(steps) = wizard["steps"].as_array_mut() {
        for step in steps.iter_mut() {
            if step["id"] == json!(step_id) {
                step["status"] = json!(status);
                step["updatedAt"] = json!(now());
            }
        }
    }
    wizard
}

/// 机器校验某步完成条件。返回 (met, detail)。
async fn evaluate_check(state: &AppState, project_id: &str, step_id: &str) -> (bool, Value) {
    let Some(template) = STEPS.iter().find(|step| step.id == step_id) else {
        return (false, json!({"reason": "unknown_step"}));
    };
    match template.check {
        CheckKind::Manual => (false, json!({"reason": "manual"})),
        CheckKind::HasSource => {
            let items = project_resources(state, project_id, "source").await;
            let count = items.as_array().map(|a| a.len()).unwrap_or(0);
            (
                count > 0,
                json!({"reason": "need_source", "count": count, "where": "项目 → 信源"}),
            )
        }
        CheckKind::CollectionVerified => {
            let plans = crate::collection::plan_rows(state)
                .await
                .unwrap_or_default();
            let items = project_resources(state, project_id, "source").await;
            let ids: Vec<&str> = items
                .as_array()
                .map(|a| a.iter().filter_map(|item| item["id"].as_str()).collect())
                .unwrap_or_default();
            let verified = plans.iter().any(|plan| {
                let id = plan["id"].as_str().unwrap_or_default();
                ids.contains(&id)
                    && plan["health"]
                        .pointer("/healthStatus")
                        .and_then(Value::as_str)
                        == Some("healthy")
                    && plan["lastRunAt"].is_string()
            });
            (
                verified,
                json!({"reason": "need_collection_run", "where": "采集 → 运行一次并成功"}),
            )
        }
        CheckKind::HasModel => {
            let models = crate::settings::effective_values(state)
                .await
                .ok()
                .and_then(|values| values.get("models").cloned())
                .unwrap_or(Value::Null);
            // 模型是系统能力：settings.models 列表或 model 文档任一存在即满足。
            let model_docs = state
                .store
                .list_documents("model")
                .await
                .map(|docs| docs.len())
                .unwrap_or(0);
            let has = model_docs > 0
                || models
                    .as_array()
                    .map(|list| !list.is_empty())
                    .unwrap_or(false);
            (has, json!({"reason": "need_model", "where": "设置 → 模型"}))
        }
        CheckKind::HasOutput => {
            let mut count = 0usize;
            for kind in ["workflow_version", "publication"] {
                count += project_resources(state, project_id, kind)
                    .await
                    .as_array()
                    .map(|a| a.len())
                    .unwrap_or(0);
            }
            (
                count > 0,
                json!({"reason": "need_output", "where": "加工 → 产出"}),
            )
        }
    }
}

/// 项目已归属（含默认项目隐式承接）的资源 id 列表。
async fn project_resources(state: &AppState, project_id: &str, kind: &str) -> Value {
    let query = format!("/projects/{project_id}/resources?kind={kind}");
    let _ = &query;
    // 直接复用 projects 的内部逻辑而非 HTTP 自调用。
    let (project, _) = match crate::projects::project_view(state, project_id).await {
        Ok(view) => view,
        Err(_) => return json!([]),
    };
    let is_default = project["isDefault"].as_bool().unwrap_or(false);
    let bindings = match state
        .store
        .list_documents(crate::projects::BINDING_KIND)
        .await
    {
        Ok(bindings) => bindings,
        Err(_) => return json!([]),
    };
    let mut ids = Vec::new();
    let mut bound = std::collections::HashSet::new();
    for doc in &bindings {
        if doc.body["resourceKind"] == json!(kind) {
            if let Some(id) = doc.body["resourceId"].as_str() {
                bound.insert(id.to_string());
            }
        }
        if doc.body["projectId"] == json!(project_id) && doc.body["resourceKind"] == json!(kind) {
            if let Some(id) = doc.body["resourceId"].as_str() {
                ids.push(json!(id));
            }
        }
    }
    if is_default && kind == "source" {
        if let Ok(sources) = crate::catalog::source_rows(state).await {
            for source in sources {
                let slug = source["id"].as_str().unwrap_or_default();
                if !slug.is_empty() && !bound.contains(slug) {
                    ids.push(json!(slug));
                }
            }
        }
    }
    json!(ids)
}

async fn check_step(
    State(state): State<Shared>,
    Path((project_id, step_id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    step_position(&step_id)?;
    let wizard = wizard_doc(&state, &project_id)
        .await?
        .ok_or_else(|| ApiError::not_found("向导"))?;
    let (met, detail) = evaluate_check(&state, &project_id, &step_id).await;
    let mut wizard = wizard.clone();
    if met {
        wizard = mark_step(&wizard, &step_id, "done");
    }
    let saved = save_wizard(&state, &project_id, wizard).await?;
    // 全部环节完成/跳过后向导收口。
    let saved = settle(&state, &project_id, saved).await?;
    Ok(util::ok(&json!({
        "wizard": saved,
        "stepId": step_id,
        "met": met,
        "detail": detail,
    })))
}

/// 收口：所有环节 done/skipped 时向导标记 completed。
async fn settle(state: &AppState, project_id: &str, mut wizard: Value) -> Result<Value, ApiError> {
    let all_settled = wizard["steps"]
        .as_array()
        .map(|steps| {
            steps
                .iter()
                .all(|step| matches!(step["status"].as_str(), Some("done") | Some("skipped")))
        })
        .unwrap_or(false);
    if all_settled && wizard["status"] != json!("completed") {
        wizard["status"] = json!("completed");
        wizard = save_wizard(state, project_id, wizard).await?;
    }
    Ok(wizard)
}

async fn complete_step(
    State(state): State<Shared>,
    Path((project_id, step_id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    step_position(&step_id)?;
    let wizard = wizard_doc(&state, &project_id)
        .await?
        .ok_or_else(|| ApiError::not_found("向导"))?;
    ensure_order(&wizard, &step_id)?;
    let wizard = save_wizard(&state, &project_id, mark_step(&wizard, &step_id, "done")).await?;
    let wizard = settle(&state, &project_id, wizard).await?;
    Ok(util::ok(&wizard))
}

async fn skip_step(
    State(state): State<Shared>,
    Path((project_id, step_id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    step_position(&step_id)?;
    let wizard = wizard_doc(&state, &project_id)
        .await?
        .ok_or_else(|| ApiError::not_found("向导"))?;
    ensure_order(&wizard, &step_id)?;
    let wizard = save_wizard(&state, &project_id, mark_step(&wizard, &step_id, "skipped")).await?;
    let wizard = settle(&state, &project_id, wizard).await?;
    Ok(util::ok(&wizard))
}

/// 高级用户跳过整个向导直接进控制台。
async fn dismiss_wizard(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
) -> Result<Response, ApiError> {
    let wizard = wizard_doc(&state, &project_id).await?.unwrap_or_else(|| {
        json!({
            "projectId": project_id,
            "templateId": "standard",
            "steps": fresh_steps(),
        })
    });
    let mut wizard = wizard;
    for step in wizard["steps"].as_array_mut().into_iter().flatten() {
        if step["status"] == json!("pending") {
            step["status"] = json!("skipped");
        }
    }
    wizard["status"] = json!("dismissed");
    let saved = save_wizard(&state, &project_id, wizard).await?;
    Ok(util::ok(&saved))
}
