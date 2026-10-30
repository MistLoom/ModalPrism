//! 建项目起步清单：只读、实时计算，不再阻塞任何操作。
//!
//! 历史上这是一台 7 步顺序状态机（建项目 → 场景模板 → 配信源 → 校验采集 →
//! 模型校验 → 投递目标 → 对外调用），必须按序完成或跳过才能推进，把"用项目"
//! 这件本应一步到位的事变成了流程负担。现在只保留同一组环节作为**建议**：
//! 每个环节的完成度按当前数据实时算出，供控制台展示一张可折叠的起步清单。

use axum::Router;
use axum::extract::{Path, State};
use axum::response::Response;
use axum::routing::get;
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::util;
use crate::{AppState, Shared};

/// 环节模板（场景级）。`check` 决定完成度如何实时计算。
struct StepTemplate {
    id: &'static str,
    title: &'static str,
    description: &'static str,
    /// 无自动条件的环节只作提示，不计入"待办"。
    optional: bool,
    check: CheckKind,
}

#[derive(Clone, Copy, PartialEq)]
enum CheckKind {
    /// 无自动条件，仅作提示。
    Manual,
    /// 项目下至少归属一个信源。
    HasSource,
    /// 至少一个信源健康状态非初始（采集已验证）。
    CollectionVerified,
    /// 系统设置里已配置至少一个模型。
    HasModel,
    /// 项目下已有投递产出（投递记录或自动投递规则）。
    HasOutput,
}

const STEPS: &[StepTemplate] = &[
    StepTemplate {
        id: "create_project",
        title: "建项目",
        description: "项目已创建，可直接开始配置信源。",
        optional: false,
        check: CheckKind::Manual,
    },
    StepTemplate {
        id: "scenario_template",
        title: "选择场景模板",
        description: "可从环节模板选一条主线，也可以直接自建。",
        optional: true,
        check: CheckKind::Manual,
    },
    StepTemplate {
        id: "configure_sources",
        title: "配置信源",
        description: "为本项目新建或共享引用至少一个信源。",
        optional: false,
        check: CheckKind::HasSource,
    },
    StepTemplate {
        id: "verify_collection",
        title: "校验采集",
        description: "任一信源完成一次健康采集。",
        optional: true,
        check: CheckKind::CollectionVerified,
    },
    StepTemplate {
        id: "model_gate",
        title: "模型与依赖",
        description: "系统设置中配置模型，投递到模型时引用。",
        optional: true,
        check: CheckKind::HasModel,
    },
    StepTemplate {
        id: "delivery_output",
        title: "投递与产出",
        description: "新增至少一条投递记录，或为一个计划配置投递目标。",
        optional: false,
        check: CheckKind::HasOutput,
    },
    StepTemplate {
        id: "api_surface",
        title: "对外调用说明",
        description: "对外 HTTP/CLI/MCP 表面与读写边界由系统统一提供。",
        optional: true,
        check: CheckKind::Manual,
    },
];

pub fn routes() -> Router<Shared> {
    Router::new().route("/projects/{projectId}/wizard", get(get_wizard))
}

/// 机器校验某步完成条件。返回 (met, hint)。
async fn evaluate_check(state: &AppState, project_id: &str, step_id: &str) -> (bool, String) {
    let Some(template) = STEPS.iter().find(|step| step.id == step_id) else {
        return (false, "未知环节".to_string());
    };
    match template.check {
        // 建项目：项目存在即完成；其余手动环节只作提示，不判定完成。
        CheckKind::Manual => (step_id == "create_project", String::new()),
        CheckKind::HasSource => {
            let count = owned_source_ids(state, project_id).await.len();
            (
                count > 0,
                if count > 0 {
                    String::new()
                } else {
                    "信源 → 新建（或让其他项目共享信源给你）".to_string()
                },
            )
        }
        CheckKind::CollectionVerified => {
            let ids = visible_source_ids(state, project_id).await;
            let plans = crate::collection::plan_rows(state)
                .await
                .unwrap_or_default();
            let verified = plans.iter().any(|plan| {
                let targets: Vec<String> = plan["targets"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|value| {
                        value
                            .as_str()
                            .map(|id| id.trim_start_matches("source/").to_string())
                    })
                    .collect();
                targets.iter().any(|id| ids.contains(id))
                    && plan["health"].as_str() == Some("healthy")
                    && plan["lastRunAt"].is_string()
            });
            (
                verified,
                if verified {
                    String::new()
                } else {
                    "采集 → 运行一次并成功".to_string()
                },
            )
        }
        CheckKind::HasModel => {
            let models = crate::settings::effective_values(state)
                .await
                .ok()
                .and_then(|values| values.get("models").cloned())
                .unwrap_or(Value::Null);
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
            (
                has,
                if has {
                    String::new()
                } else {
                    "设置 → 模型".to_string()
                },
            )
        }
        CheckKind::HasOutput => {
            let records = state
                .store
                .list_documents("delivery_record")
                .await
                .map(|docs| docs.len())
                .unwrap_or(0);
            let rules = state
                .store
                .list_documents("delivery_rule")
                .await
                .map(|docs| docs.len())
                .unwrap_or(0);
            let count = records + rules;
            (
                count > 0,
                if count > 0 {
                    String::new()
                } else {
                    "内容投递 → 新建规则或手动投递".to_string()
                },
            )
        }
    }
}

/// 本项目**归属**的信源 id（不含他项目共享来的）。
async fn owned_source_ids(state: &AppState, project_id: &str) -> Vec<String> {
    crate::catalog::source_rows(state)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|row| row["project"].as_str() == Some(project_id))
        .filter_map(|row| row["id"].as_str().map(str::to_string))
        .collect()
}

/// 本项目**可见**的信源 id（归属 + 共享）。
async fn visible_source_ids(state: &AppState, project_id: &str) -> Vec<String> {
    crate::catalog::source_rows(state)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|row| crate::catalog::source_visible(row, project_id))
        .filter_map(|row| row["id"].as_str().map(str::to_string))
        .collect()
}

/// 只读起步清单：每个环节的实时完成度 + 未满足时去哪儿做。
async fn get_wizard(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
) -> Result<Response, ApiError> {
    crate::projects::project_view(&state, &project_id).await?;
    let mut steps = Vec::with_capacity(STEPS.len());
    for step in STEPS {
        let (met, hint) = evaluate_check(&state, &project_id, step.id).await;
        let status = if met {
            "done"
        } else if step.optional {
            "optional"
        } else {
            "pending"
        };
        steps.push(json!({
            "id": step.id,
            "title": step.title,
            "description": step.description,
            "status": status,
            "met": met,
            "optional": step.optional,
            "hint": hint,
        }));
    }
    let pending = steps
        .iter()
        .filter(|step| step["status"] == json!("pending"))
        .count();
    Ok(util::ok(&json!({
        "projectId": project_id,
        "status": if pending == 0 { "ready" } else { "active" },
        "pending": pending,
        "steps": steps,
    })))
}
