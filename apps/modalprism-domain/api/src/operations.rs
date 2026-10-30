//! 持久化 Operation、任务、队列与健康观测。

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Value, json};
use strata_client::{TaskFilter, TaskRow};

use crate::error::ApiError;
use crate::util::{self, cursor, cursor_offset, page, page_total};
use crate::{AppState, Shared, new_id, now};

/// 任务列表的一次性扫描上限：页码分页的 total 以此为封顶口径。
const QUEUE_SCAN: usize = 1_000;

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/operations/tasks", get(list_tasks))
        .route("/operations/tasks/batch", post(batch_tasks))
        .route("/operations/tasks/purge", post(purge_tasks))
        .route("/operations/tasks/{taskId}/run", post(run_task))
        .route("/operations/queues", get(list_queues))
        .route("/operations/queues/{queueId}/pause", post(pause_queue))
        .route("/operations/queues/{queueId}/resume", post(resume_queue))
        .route("/operations/derivations/run", post(run_derivations))
        .route("/operations/health", get(current_health))
        .route("/operations/healthHistory", get(list_health_history))
        .route("/operations/healthRecomputations", post(recompute_health))
        .route("/operations/{operationId}", get(get_operation))
        .route("/operations/{operationId}/events", get(operation_events))
        .route("/operations/{operationId}/cancel", post(cancel_operation))
        .route("/operations/{operationId}/retry", post(retry_operation))
        .route("/operations", get(list_operations))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListQuery {
    #[serde(default)]
    limit: Option<String>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    offset: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    source_id: Option<String>,
    /// `all` 取消项目隔离过滤（管理与迁移用）。
    #[serde(default)]
    scope: Option<String>,
}

/// 异步动作的统一入口：先落 Operation，再交给后台 worker。
pub async fn start(
    state: &Arc<AppState>,
    kind: &str,
    resource_refs: Vec<Value>,
    action: Value,
) -> Result<Value, ApiError> {
    let operation = json!({
        "operationId": new_id("op"),
        "kind": kind,
        "status": "queued",
        "progress": 0.0,
        "cancellable": true,
        "retryable": false,
        "retryOf": Value::Null,
        "resourceRefs": resource_refs,
        "resultSummary": Value::Null,
        "errorSummary": Value::Null,
        "createdAt": now(),
        "completedAt": Value::Null,
    });
    let id = operation["operationId"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    state
        .store
        .insert_operation(&operation, &action, &crate::error::current_request_id())
        .await?;
    state
        .store
        .append_operation_event(&id, "queued", "操作已接受", None)
        .await?;
    let state = Arc::clone(state);
    tokio::spawn(async move {
        run(state, id, action).await;
    });
    Ok(operation)
}

pub fn accepted(operation: &Value) -> Response {
    let id = operation["operationId"].as_str().unwrap_or_default();
    let mut response = (StatusCode::ACCEPTED, Json(operation.clone())).into_response();
    if let Ok(location) = format!("/api/v1/operations/{id}").parse() {
        response.headers_mut().insert(header::LOCATION, location);
    }
    response
}

async fn run(state: Shared, id: String, action: Value) {
    if cancelled(&state, &id).await {
        finish(&state, &id, "cancelled", 0.0, None, None, false).await;
        return;
    }
    update(&state, &id, "running", 0.1).await;
    let kind = action
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let outcome = match kind.as_str() {
        "source.delete" => crate::catalog::execute_source_delete(&state, &id, &action).await,
        "source.probe" => crate::catalog::execute_source_probe(&state, &id, &action).await,
        "source.analysis" => crate::catalog::execute_source_analysis(&state, &id, &action).await,
        "binding.rebind" => crate::bindings::execute_binding_rebind(&state, &id, &action).await,
        "adapter.draft" => crate::catalog::execute_adapter_draft(&state, &id, &action).await,
        "adapter.approve" => crate::catalog::execute_adapter_approve(&state, &id, &action).await,
        "collection.run" => crate::collection::execute_run(&state, &id, &action).await,
        "content.batch" => crate::content::execute_batch(&state, &id, &action).await,
        "content.replay" => crate::content::execute_replay(&state, &id, &action).await,
        "delivery.manual" => crate::delivery::execute_manual(&state, &id, &action).await,
        "channel.test" => crate::channels::execute_channel_test(&state, &id, &action).await,
        "egressHost.reset" => crate::channels::execute_egress_reset(&state, &id, &action).await,
        "settings.apply" => crate::settings::execute_apply(&state, &id, &action).await,
        "settings.rollback" => crate::settings::execute_rollback(&state, &id, &action).await,
        "health.recompute" => execute_health(&state, &id, &action).await,
        "logs.export" => crate::observe::execute_export(&state, &id, &action).await,
        "tasks.batch" => execute_task_batch(&state, &id, &action).await,
        "task.run" => execute_task_run(&state, &id, &action).await,
        "derivation.run" => execute_derivation_run(&state, &id, &action).await,
        "maintenance.reset" => crate::maintenance::execute_reset(&state, &id, &action).await,
        "maintenance.factoryReset" => {
            crate::maintenance::execute_factory_reset(&state, &id, &action).await
        }
        other => Err(ApiError::internal(format!("未知操作类型: {other}"))),
    };
    if cancelled(&state, &id).await {
        finish(&state, &id, "cancelled", 1.0, None, None, false).await;
        return;
    }
    match outcome {
        Ok(result) => {
            let partial = result
                .get("partial")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let status = if partial {
                "partiallySucceeded"
            } else {
                "succeeded"
            };
            finish(&state, &id, status, 1.0, Some(result.clone()), None, partial).await;
            notify_finished(&state, &action, &kind, status, Some(&result), None).await;
        }
        Err(error) => {
            let retryable = error.retryable;
            let summary = error.body()["error"].clone();
            // 校验类错误把字段内的说明当原因：通知里不需要「sourceId: 」这类前缀。
            let message = summary["fieldErrors"][0]["message"]
                .as_str()
                .unwrap_or(&error.message)
                .to_string();
            finish(
                &state,
                &id,
                "failed",
                1.0,
                None,
                Some(summary),
                retryable,
            )
            .await;
            notify_finished(&state, &action, &kind, "failed", None, Some(message)).await;
        }
    }
}

/// 一条终态通知。
struct Notice {
    category: &'static str,
    event: &'static str,
    title: String,
    message: String,
    link: Option<String>,
}

/// 终态操作 → 通知：把用户关心的动作翻成具体事件，其余只留日志。
///
/// Operation 成功不等于业务成功：采集要看任务终态、投递要看记录结果，所以这里
/// 还会读取 `result` 里的细分状态，避免把「已提交」报成「已完成」。操作级失败没有
/// `result`，信源与原因改从发起时的 `action` 和错误信息里回读。
async fn notify_finished(
    state: &AppState,
    action: &Value,
    kind: &str,
    status: &str,
    result: Option<&Value>,
    error: Option<String>,
) {
    let failed = status == "failed";
    let partial = status == "partiallySucceeded";
    let level = if failed || partial { "warn" } else { "info" };
    let detail = error.unwrap_or_default();

    let notice = match kind {
        "collection.run" => {
            // 采集可能根本没轮到建任务（信源不可采集），这时的信源只在 action 里。
            let source_id = result
                .and_then(|value| value["sourceId"].as_str())
                .or_else(|| action["sourceId"].as_str())
                .unwrap_or_default()
                .to_string();
            let label = source_label(state, &source_id).await;
            let task_status = result
                .and_then(|value| value["status"].as_str())
                .unwrap_or_default();
            let task_error = result
                .and_then(|value| value["error"].as_str())
                .unwrap_or_default();
            let reason = if task_error.is_empty() {
                detail.as_str()
            } else {
                task_error
            };
            let link = Some(match source_id.is_empty() {
                true => "sources".to_string(),
                false => format!("sources?query={source_id}"),
            });
            collection_notice(failed, task_status, &label, reason, link)
        }
        "delivery.manual" => {
            let record_status = result
                .and_then(|value| value["status"].as_str())
                .unwrap_or_default();
            let succeeded = result
                .and_then(|value| value["succeeded"].as_u64())
                .unwrap_or(0);
            let failed_targets = result
                .and_then(|value| value["failed"].as_u64())
                .unwrap_or(0);
            let bad = failed || record_status == "failed" || (succeeded == 0 && failed_targets > 0);
            Notice {
                category: "delivery",
                event: if bad {
                    "delivery.failed"
                } else {
                    "content.delivered"
                },
                title: if bad {
                    "内容投递失败".into()
                } else {
                    "内容投递完成".into()
                },
                message: if detail.is_empty() {
                    format!("成功 {succeeded} 个目标，失败 {failed_targets} 个目标")
                } else {
                    detail.clone()
                },
                link: Some("delivery?tab=records".into()),
            }
        }
        "channel.test" => Notice {
            category: "delivery",
            event: if failed {
                "delivery.failed"
            } else {
                "system.notice"
            },
            title: if failed {
                "渠道连通检测失败".into()
            } else {
                "渠道连通检测完成".into()
            },
            message: if detail.is_empty() {
                "渠道可正常发送".into()
            } else {
                detail.clone()
            },
            link: Some("notifications?tab=push".into()),
        },
        "source.probe" | "source.analysis" => Notice {
            category: "source",
            event: if failed {
                "source.unhealthy"
            } else {
                "system.notice"
            },
            title: match (kind, failed) {
                ("source.probe", true) => "信源探测失败".into(),
                ("source.probe", false) => "信源探测完成".into(),
                (_, true) => "信源分析失败".into(),
                (_, false) => "信源分析完成".into(),
            },
            message: if detail.is_empty() {
                "访问链路可用".into()
            } else {
                detail.clone()
            },
            link: Some("sources".into()),
        },
        "settings.apply" | "settings.rollback" => {
            let apply = kind == "settings.apply";
            let generation = result
                .and_then(|value| value["generation"].as_u64())
                .unwrap_or(0);
            let applied = result
                .and_then(|value| value["applied"].as_array())
                .map_or(0, Vec::len);
            let restart = result
                .and_then(|value| value["restartRequired"].as_array())
                .map_or(0, Vec::len);
            Notice {
                category: "system",
                event: if failed {
                    "system.notice"
                } else {
                    "settings.changed"
                },
                title: match (apply, failed) {
                    (true, true) => "配置应用失败".into(),
                    (true, false) => "配置已更新".into(),
                    (false, true) => "配置回滚失败".into(),
                    (false, false) => "配置已回滚".into(),
                },
                message: if !detail.is_empty() {
                    detail.clone()
                } else {
                    format!(
                        "generation {generation} · {applied} 项已生效{}",
                        if restart > 0 {
                            format!("，{restart} 项需重启生效")
                        } else {
                            String::new()
                        }
                    )
                },
                link: Some("settings".into()),
            }
        }
        "content.batch" | "content.replay" | "derivation.run" => Notice {
            category: "content",
            event: if failed {
                "system.notice"
            } else {
                "content.produced"
            },
            title: match (kind, failed) {
                ("content.batch", true) => "内容批量操作失败".into(),
                ("content.batch", false) => "内容批量操作完成".into(),
                ("content.replay", true) => "内容回放失败".into(),
                ("content.replay", false) => "内容回放完成".into(),
                (_, true) => "派生执行失败".into(),
                (_, false) => "派生执行完成".into(),
            },
            message: if detail.is_empty() {
                "已写入内容库".into()
            } else {
                detail.clone()
            },
            link: Some("content".into()),
        },
        "source.delete" | "adapter.draft" | "adapter.approve" | "binding.rebind"
        | "tasks.batch" | "egressHost.reset" => Notice {
            category: "system",
            event: "system.notice",
            title: format!(
                "{}{}",
                operation_label(kind),
                if failed { "失败" } else { "完成" }
            ),
            message: if detail.is_empty() {
                "操作已结束".into()
            } else {
                detail.clone()
            },
            link: Some(match kind {
                "source.delete" | "adapter.draft" | "adapter.approve" | "binding.rebind" => {
                    "sources".into()
                }
                "tasks.batch" => "tasks".into(),
                _ => "logs".into(),
            }),
        },
        "maintenance.reset" | "maintenance.factoryReset" => Notice {
            category: "system",
            event: "maintenance.reset",
            title: match (kind, failed) {
                ("maintenance.factoryReset", true) => "恢复出厂失败".into(),
                ("maintenance.factoryReset", false) => "恢复出厂完成".into(),
                (_, true) => "数据清理失败".into(),
                (_, false) => "数据清理完成".into(),
            },
            message: if detail.is_empty() {
                "业务数据已按所选范围清理".into()
            } else {
                detail.clone()
            },
            link: Some("logs?tab=audit".into()),
        },
        _ => {
            // 未映射的操作只在失败时提醒：成功的只读/运维动作不占通知中心。
            if !failed {
                return;
            }
            Notice {
                category: "system",
                event: "system.notice",
                title: format!("{}失败", operation_label(kind)),
                message: detail,
                link: None,
            }
        }
    };
    crate::notifications::emit(
        state,
        notice.category,
        notice.event,
        level,
        notice.title,
        notice.message,
        notice.link.as_deref(),
    )
    .await;
}

/// 采集操作的终态 → 通知。
///
/// 任务终态优先；操作自己在建任务之前就失败时（信源没有采集入口）任务状态是空的，
/// 这时用错误原因兜底，至少告诉用户是哪个信源没采起来、为什么。
fn collection_notice(
    failed: bool,
    task_status: &str,
    label: &str,
    reason: &str,
    link: Option<String>,
) -> Notice {
    match task_status {
        "done" => Notice {
            category: "source",
            event: "collection.completed",
            title: format!("采集完成：{label}"),
            message: "采集结果已入库".into(),
            link,
        },
        "failed" => Notice {
            category: "source",
            event: "collection.failed",
            title: format!("采集失败：{label}"),
            message: if reason.is_empty() {
                "任务以失败结束".into()
            } else {
                reason.to_string()
            },
            link,
        },
        _ if failed => Notice {
            category: "source",
            event: "collection.failed",
            title: if label.is_empty() {
                "采集失败".into()
            } else {
                format!("采集失败：{label}")
            },
            message: if reason.is_empty() {
                "采集未能开始".into()
            } else {
                reason.to_string()
            },
            link,
        },
        _ => Notice {
            category: "source",
            event: "collection.started",
            title: format!("采集已提交：{label}"),
            message: "任务已入队，完成后会再通知一次".into(),
            link,
        },
    }
}

fn operation_label(kind: &str) -> &str {
    match kind {
        "source.delete" => "删除信源",
        "source.probe" => "信源探测",
        "source.analysis" => "信源分析",
        "binding.rebind" => "能力重绑",
        "adapter.draft" => "适配器草拟",
        "adapter.approve" => "适配器批准",
        "collection.run" => "采集",
        "content.batch" => "内容批量操作",
        "content.replay" => "内容回放",
        "delivery.manual" => "内容投递",
        "channel.test" => "渠道连通检测",
        "egressHost.reset" => "出站主机复位",
        "settings.apply" => "配置应用",
        "settings.rollback" => "配置回滚",
        "health.recompute" => "健康重算",
        "logs.export" => "日志导出",
        "tasks.batch" => "任务批量操作",
        "task.run" => "任务执行",
        "derivation.run" => "派生执行",
        "maintenance.reset" => "数据清理",
        "maintenance.factoryReset" => "恢复出厂",
        other => other,
    }
}

async fn source_label(state: &AppState, source_id: &str) -> String {
    let bare = source_id.trim_start_matches("source/");
    crate::catalog::source_rows(state)
        .await
        .ok()
        .and_then(|rows| {
            rows.into_iter()
                .find(|row| row["id"].as_str() == Some(bare))
                .and_then(|row| row["name"].as_str().map(str::to_string))
        })
        .unwrap_or_else(|| bare.to_string())
}

async fn update(state: &AppState, id: &str, status: &str, progress: f64) {
    let _ = state
        .store
        .update_operation(
            id,
            &json!({
                "status": status,
                "progress": progress,
                "cancellable": matches!(status, "queued" | "running"),
            }),
        )
        .await;
}

/// 阶段性进度：长任务（清空/恢复出厂）在关键阶段推进百分比。
///
/// 控制台的任务卡片只认 `progress`：不推进就会从头到尾停在起始值，看起来像没在跑。
pub(crate) async fn progress(state: &AppState, id: &str, value: f64) {
    update(state, id, "running", value.clamp(0.0, 0.99)).await;
}

async fn finish(
    state: &AppState,
    id: &str,
    status: &str,
    progress: f64,
    result: Option<Value>,
    error: Option<Value>,
    retryable: bool,
) {
    let patch = json!({
        "status": status,
        "progress": progress,
        "cancellable": false,
        "retryable": retryable || matches!(status, "failed" | "cancelled" | "partiallySucceeded"),
        "resultSummary": result,
        "errorSummary": error,
        "completedAt": now(),
    });
    let _ = state.store.update_operation(id, &patch).await;
    let _ = state
        .store
        .append_operation_event(id, status, "操作结束", result.as_ref())
        .await;
    let level = if status == "failed" { "error" } else { "info" };
    // 落库文案用字典里的用户语言，避免日志里出现 `操作 failed` 这类裸程序值。
    let status_label = dictionary::lookup("task.status", status)
        .map(|entry| entry.label)
        .unwrap_or(status);
    let _ = crate::observe::append_log(
        state,
        level,
        "operation",
        &format!("操作{status_label}"),
        json!({"status": status, "result": result, "error": error}),
        Some(id),
    )
    .await;
}

pub async fn cancelled(state: &AppState, id: &str) -> bool {
    state
        .store
        .get_operation(id)
        .await
        .ok()
        .flatten()
        .and_then(|stored| stored.operation["status"].as_str().map(str::to_string))
        .is_some_and(|status| status == "cancelling" || status == "cancelled")
}

pub async fn check_cancel(state: &AppState, id: &str) -> Result<(), ApiError> {
    if cancelled(state, id).await {
        Err(ApiError::conflict("操作已取消"))
    } else {
        Ok(())
    }
}

/// 进程重启后恢复未完成的操作：清空/恢复出厂与排期发布继续执行，
/// 其余无法安全重放的操作标记失败并保留手动重试入口。
pub async fn recover_interrupted(state: std::sync::Arc<AppState>) {
    let Ok(operations) = state.store.list_operations(None, 500).await else {
        return;
    };
    for operation in operations {
        let status = operation["status"].as_str().unwrap_or_default();
        if !matches!(status, "queued" | "running" | "cancelling") {
            continue;
        }
        let Some(id) = operation["operationId"].as_str().map(str::to_string) else {
            continue;
        };
        let Ok(Some(stored)) = state.store.get_operation(&id).await else {
            continue;
        };
        let kind = stored.action["kind"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if matches!(
            kind.as_str(),
            "maintenance.reset" | "maintenance.factoryReset"
        ) {
            let _ = state.resetting.compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            );
            let task_state = std::sync::Arc::clone(&state);
            tokio::spawn(async move {
                run(task_state, id, stored.action).await;
            });
            continue;
        }
        let failed = status != "cancelling";
        let error = failed.then(|| {
            json!({
                "code": "interrupted",
                "message": "进程重启，操作已中断，可手动重试",
                "retryable": true,
            })
        });
        finish(
            &state,
            &id,
            if failed { "failed" } else { "cancelled" },
            1.0,
            None,
            error,
            true,
        )
        .await;
    }
}

pub(crate) async fn get_operation(
    State(state): State<Shared>,
    Path(operation_id): Path<String>,
) -> Result<Response, ApiError> {
    let stored = state
        .store
        .get_operation(&operation_id)
        .await?
        .ok_or_else(|| ApiError::not_found(&operation_id))?;
    Ok(util::ok(&stored.operation))
}

pub(crate) async fn list_operations(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let filter = format!("operations:{}", query.status.clone().unwrap_or_default());
    let offset = cursor_offset(query.cursor.as_deref(), &filter)?;
    let status = query
        .status
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let mut items = state
        .store
        .list_operations(status, limit + offset + 1)
        .await?;
    items.drain(..offset.min(items.len()));
    let next = if items.len() > limit {
        Some(cursor(&filter, offset + limit))
    } else {
        None
    };
    items.truncate(limit);
    Ok(util::ok(&page(items, next)))
}

pub(crate) async fn operation_events(
    State(state): State<Shared>,
    Path(operation_id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let after = query
        .cursor
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("0")
        .parse::<i64>()
        .map_err(|_| ApiError::cursor_expired())?;
    let mut items = state
        .store
        .list_operation_events(&operation_id, after, limit)
        .await?;
    let next = if items.len() > limit {
        items
            .get(limit - 1)
            .and_then(|item| item["cursor"].as_str())
            .map(str::to_string)
    } else {
        None
    };
    items.truncate(limit);
    Ok(util::ok(&page(items, next)))
}

pub(crate) async fn cancel_operation(
    State(state): State<Shared>,
    Path(operation_id): Path<String>,
    _headers: HeaderMap,
) -> Result<Response, ApiError> {
    let stored = state
        .store
        .get_operation(&operation_id)
        .await?
        .ok_or_else(|| ApiError::not_found(&operation_id))?;
    let status = stored.operation["status"].as_str().unwrap_or("failed");
    if matches!(
        status,
        "succeeded" | "partiallySucceeded" | "failed" | "cancelled"
    ) {
        return Err(ApiError::not_cancellable());
    }
    if !stored.operation["cancellable"].as_bool().unwrap_or(false) {
        return Err(ApiError::not_cancellable());
    }
    state
        .store
        .update_operation(
            &operation_id,
            &json!({"status": "cancelling", "cancellable": false}),
        )
        .await?;
    state
        .store
        .append_operation_event(&operation_id, "cancel", "用户请求取消", None)
        .await?;
    let stored = state
        .store
        .get_operation(&operation_id)
        .await?
        .ok_or_else(|| ApiError::not_found(&operation_id))?;
    Ok(accepted(&stored.operation))
}

pub(crate) async fn retry_operation(
    State(state): State<Shared>,
    Path(operation_id): Path<String>,
    Json(_body): Json<Value>,
) -> Result<Response, ApiError> {
    let stored = state
        .store
        .get_operation(&operation_id)
        .await?
        .ok_or_else(|| ApiError::not_found(&operation_id))?;
    let retryable = stored.operation["retryable"].as_bool().unwrap_or(false);
    if !retryable {
        return Err(ApiError::not_retryable());
    }
    let refs = stored.operation["resourceRefs"].clone();
    let kind = stored.operation["kind"]
        .as_str()
        .unwrap_or("operation.retry")
        .to_string();
    let mut action = stored.action.clone();
    if let Value::Object(map) = &mut action {
        map.insert("retryOf".into(), Value::String(operation_id.clone()));
    }
    let mut operation = start(
        &state,
        &kind,
        refs.as_array().cloned().unwrap_or_default(),
        action,
    )
    .await?;
    if let Some(map) = operation.as_object_mut() {
        map.insert("retryOf".into(), Value::String(operation_id.clone()));
    }
    let id = operation["operationId"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    state
        .store
        .update_operation(&id, &json!({"retryOf": operation_id.clone()}))
        .await?;
    Ok(accepted(&operation))
}

async fn execute_health(
    state: &Arc<AppState>,
    _id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let source_ids: Vec<String> = action
        .get("sourceIds")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    state.services.health_evaluate().await?;
    let history = health_snapshot(state, &source_ids).await?;
    state
        .store
        .insert_document("health_history", &new_id("h"), &history)
        .await?;
    Ok(
        json!({"partial": false, "sources": history["sources"], "evaluatedAt": history["evaluatedAt"]}),
    )
}

async fn health_snapshot(state: &AppState, source_filter: &[String]) -> Result<Value, ApiError> {
    let statuses = state
        .services
        .planner_statuses()
        .await
        .map_err(ApiError::from)?;
    let mut sources = Vec::new();
    for (source, status) in statuses {
        if !source_filter.is_empty() && !source_filter.contains(&source) {
            continue;
        }
        sources.push(json!({
            "sourceId": source,
            "status": status.health_status,
            "reason": status.last_error,
            "lastOkAt": status.last_ok_at,
            "todayYield": status.today_yield,
            "alert": status.alert,
        }));
    }
    let queues = state
        .services
        .queue_states()
        .await
        .map_err(ApiError::from)?;
    let counts = state
        .services
        .store_counts()
        .await
        .map_err(ApiError::from)?;
    let egress = state
        .services
        .egress_report()
        .await
        .map_err(ApiError::from)?;
    Ok(json!({
        "evaluatedAt": now(),
        "status": if sources.iter().any(|item| item["status"] == "failed") { "degraded" } else { "online" },
        "sources": sources,
        "queues": serde_json::to_value(queues).unwrap_or(Value::Null),
        "storage": serde_json::to_value(counts).unwrap_or(Value::Null),
        "egress": serde_json::to_value(egress).unwrap_or(Value::Null),
    }))
}

async fn current_health(State(state): State<Shared>) -> Result<Response, ApiError> {
    let snapshot = health_snapshot(&state, &[]).await?;
    Ok(util::ok(&snapshot))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryQuery {
    #[serde(default)]
    limit: Option<String>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    offset: Option<String>,
    #[serde(default)]
    source_id: Option<String>,
}

async fn list_health_history(
    State(state): State<Shared>,
    Query(query): Query<HistoryQuery>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let filter = format!("health:{}", query.source_id.clone().unwrap_or_default());
    let offset = util::page_offset(query.offset.as_deref(), query.cursor.as_deref(), &filter)?;
    let docs = state.store.list_documents("health_history").await?;
    let retention = crate::settings::value(&state, "retention.healthHistoryLimit")
        .await?
        .as_u64()
        .unwrap_or(200)
        .clamp(10, 5000) as usize;
    let mut docs = docs;
    docs.sort_by(|left, right| {
        right.body["evaluatedAt"]
            .as_str()
            .unwrap_or("")
            .cmp(left.body["evaluatedAt"].as_str().unwrap_or(""))
    });
    docs.truncate(retention);
    let total = docs.len();
    let mut items: Vec<Value> = docs
        .into_iter()
        .skip(offset)
        .take(limit + 1)
        .map(|doc| doc.body)
        .collect();
    if let Some(source) = query.source_id.as_deref().filter(|value| !value.is_empty()) {
        for item in &mut items {
            if let Some(list) = item.get_mut("sources").and_then(Value::as_array_mut) {
                list.retain(|entry| entry["sourceId"] == source);
            }
        }
    }
    let more = items.len() > limit;
    let items: Vec<Value> = items.into_iter().take(limit).collect();
    let next = if more || offset + items.len() < total {
        Some(cursor(&filter, offset + limit))
    } else {
        None
    };
    Ok(util::ok(&page_total(items, next, total)))
}

async fn recompute_health(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["sourceIds"])?;
    let source_ids = util::optional_strings(map, "sourceIds")?.unwrap_or_default();
    let operation = start(
        &state,
        "health.recompute",
        vec![json!({"kind": "health", "id": "current"})],
        json!({"kind": "health.recompute", "sourceIds": source_ids}),
    )
    .await?;
    Ok(accepted(&operation))
}

async fn list_tasks(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let status = query
        .status
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let kind = query
        .kind
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let filter = format!(
        "tasks:{}:{}:{}",
        status.unwrap_or(""),
        query.source_id.clone().unwrap_or_default(),
        kind.unwrap_or("")
    );
    let offset = util::page_offset(query.offset.as_deref(), query.cursor.as_deref(), &filter)?;
    // 项目隔离：任务归属由自有信源的 sourceId 派生；共享信源不把他项目的任务带过来，
    // 无来源的系统任务只在 scope=all 下展示。
    let active = project.id().to_string();
    let allowed = if query.scope.as_deref() == Some("all") {
        None
    } else {
        Some(crate::catalog::owned_source_ids(&state, &active).await?)
    };
    let visible = |source_id: Option<&str>, owner: Option<&str>| -> bool {
        if allowed.is_none() {
            return true;
        }
        if let Some(owner) = owner.filter(|value| !value.is_empty()) {
            return owner == active;
        }
        match source_id.filter(|value| !value.is_empty()) {
            Some(id) => allowed.as_ref().is_some_and(|set| set.contains(id)),
            // 无来源的维护任务不归属任何项目，避免跨项目泄露：只在 scope=all 可见。
            None => false,
        }
    };
    let mut items = Vec::new();
    if kind.is_none() || kind == Some("collection") || kind == Some("derived") {
        let task_filter = TaskFilter {
            source: query.source_id.clone().filter(|value| !value.is_empty()),
            ..TaskFilter::default()
        };
        let result = state
            .services
            .queue_list(&task_filter, None, QUEUE_SCAN)
            .await
            .map_err(ApiError::from)?;
        for row in result.rows {
            let view = task_view(&row);
            if !visible(view["sourceId"].as_str(), view["project"].as_str()) {
                continue;
            }
            if status.is_none_or(|expected| view["status"] == expected) {
                items.push(view);
            }
        }
    }
    if kind.is_none() || kind == Some("maintenance") {
        for doc in state.store.list_documents("task").await? {
            if let Some(status) = status
                && doc.body["status"] != status
            {
                continue;
            }
            if let Some(want) = kind
                && doc.body["kind"] != want
            {
                continue;
            }
            if !visible(
                doc.body["sourceId"].as_str(),
                doc.body["project"].as_str(),
            ) {
                continue;
            }
            items.push(doc.body);
        }
    }
    items.sort_by(|left, right| {
        right["createdAt"]
            .as_str()
            .unwrap_or("")
            .cmp(left["createdAt"].as_str().unwrap_or(""))
    });
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

fn task_view(row: &TaskRow) -> Value {
    let status = match row.status.as_str() {
        "done" => "succeeded",
        "leased" => "running",
        // 协作取消会写入固定文案；只有携带真实错误的取消才算失败。
        "cancelled"
            if row
                .last_error
                .as_deref()
                .is_some_and(|error| !error.trim().is_empty() && error != "已取消") =>
        {
            "failed"
        }
        other => other,
    };
    let mut view = json!({
        "id": row.task_id,
        "kind": "collection",
        "status": status,
        "title": row.source_id,
        "sourceId": row.source_id,
        "planId": row.plan_id,
        "queue": row.queue,
        "reason": row.reason,
        "attempt": row.attempt,
        "createdAt": row.created_at,
        "updatedAt": row.lease_until.clone().unwrap_or_else(|| row.created_at.clone()),
        "retryAt": row.retry_at,
        "error": row.last_error,
        "locator": row.locator,
    });
    crate::dictionary::decorate(
        &mut view,
        &[
            ("kind", "task.kind"),
            ("status", "task.status"),
            ("queue", "task.queue"),
            ("reason", "task.reason"),
        ],
    );
    view
}

async fn batch_tasks(
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
    // 项目隔离：批量动作只能落到当前项目自有信源的任务上，否则按不存在处理。
    let allowed = crate::catalog::owned_source_ids(&state, project.id()).await?;
    let queued = state
        .services
        .queue_list(&TaskFilter::default(), None, QUEUE_SCAN)
        .await
        .map_err(ApiError::from)?
        .rows
        .into_iter()
        .map(|row| (row.task_id.clone(), row.source_id.clone()))
        .collect::<std::collections::HashMap<String, String>>();
    for id in &ids {
        let (owner, source) = match state.store.get_document("task", id).await? {
            Some(doc) => (
                doc.body["project"].as_str().map(str::to_string),
                doc.body["sourceId"].as_str().map(str::to_string),
            ),
            None => (None, queued.get(id).cloned()),
        };
        let visible = match (owner.as_deref(), source.as_deref()) {
            (Some(owner), _) => owner == project.id(),
            (None, Some(source)) => allowed.contains(source),
            (None, None) => false,
        };
        if !visible {
            return Err(ApiError::not_found(format!("任务 {id}")));
        }
    }
    let action = util::text(map, "action")?;
    if !matches!(action.as_str(), "cancel" | "retry" | "requeue" | "delete") {
        return Err(ApiError::validation(
            "action",
            "enum",
            "任务只支持 cancel、retry、requeue 或 delete",
        ));
    }
    let operation = start(
        &state,
        "tasks.batch",
        vec![json!({"kind": "taskBatch", "id": new_id("batch")})],
        json!({"kind": "tasks.batch", "ids": ids, "action": action}),
    )
    .await?;
    Ok(accepted(&operation))
}

async fn execute_task_batch(
    state: &AppState,
    _operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let ids = action["ids"].as_array().cloned().unwrap_or_default();
    let decision = action["action"].as_str().unwrap_or("requeue");
    let mut succeeded = Vec::new();
    let mut failed = Vec::new();
    for id in ids {
        let Some(id) = id.as_str() else { continue };
        if let Some(doc) = state.store.get_document("task", id).await? {
            if decision == "delete" {
                state.store.delete_document("task", id).await?;
                succeeded.push(json!({"id": id, "status": "deleted"}));
                continue;
            }
            let next = match decision {
                "cancel" => "cancelled",
                _ => "queued",
            };
            let mut body = doc.body;
            if let Some(map) = body.as_object_mut() {
                map.insert("status".into(), Value::String(next.into()));
                map.insert("updatedAt".into(), Value::String(now()));
            }
            state
                .store
                .update_document("task", id, &body, Some(doc.version))
                .await?;
            succeeded.push(json!({"id": id, "status": next}));
            continue;
        }
        let Ok(task_id) = strata_model::TaskId::new(id) else {
            failed.push(json!({"id": id, "code": "notFound", "message": "任务不存在"}));
            continue;
        };
        if decision == "delete" {
            let _ = state.services.queue_cancel(&task_id).await;
            match state.services.queue_remove(&task_id).await {
                Ok(true) => succeeded.push(json!({"id": id, "status": "deleted"})),
                Ok(false) => failed.push(json!({
                    "id": id,
                    "code": "notFound",
                    "message": "任务不存在"
                })),
                Err(error) => failed.push(json!({
                    "id": id,
                    "code": "dependencyUnavailable",
                    "message": error.to_string()
                })),
            }
            continue;
        }
        let outcome = match decision {
            "cancel" => state.services.queue_cancel(&task_id).await,
            _ => state.services.queue_requeue(&task_id).await,
        };
        match outcome {
            Ok(true) => succeeded.push(json!({"id": id, "status": decision})),
            Ok(false) => failed.push(json!({
                "id": id,
                "code": "conflict",
                "message": "当前状态不允许该动作"
            })),
            Err(error) => failed.push(json!({
                "id": id,
                "code": "dependencyUnavailable",
                "message": error.to_string()
            })),
        }
    }
    Ok(json!({
        "partial": !failed.is_empty() && !succeeded.is_empty(),
        "succeeded": succeeded,
        "failed": failed,
    }))
}

/// 立即执行单条任务：控制台传入任务号，不碰 spec、不拼任务体。
async fn run_task(
    State(state): State<Shared>,
    Path(task_id): Path<String>,
) -> Result<Response, ApiError> {
    if strata_model::TaskId::new(&task_id).is_err() {
        return Err(ApiError::not_found(format!("任务 {task_id}")));
    }
    let operation = start(
        &state,
        "task.run",
        vec![json!({"kind": "task", "id": task_id.clone()})],
        json!({"kind": "task.run", "taskId": task_id}),
    )
    .await?;
    Ok(accepted(&operation))
}

async fn execute_task_run(
    state: &AppState,
    _operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let id = action["taskId"].as_str().unwrap_or_default();
    let task_id =
        strata_model::TaskId::new(id).map_err(|_| ApiError::not_found(format!("任务 {id}")))?;
    let row = state.services.task_run(&task_id).await?;
    Ok(json!({
        "partial": false,
        "taskId": row.task_id,
        "status": row.status,
        "error": row.last_error,
    }))
}

/// 清空任务：可按状态、类型、来源与创建时间筛选；活动任务先取消再物理删除。
async fn purge_tasks(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["status", "kind", "sourceId", "before", "dryRun"])?;
    let status =
        util::optional_text(map, "status")?.filter(|value| !value.is_empty() && value != "all");
    if let Some(value) = status.as_deref()
        && !matches!(
            value,
            "queued" | "running" | "succeeded" | "failed" | "cancelled"
        )
    {
        return Err(ApiError::validation("status", "enum", "未知任务状态"));
    }
    let kind =
        util::optional_text(map, "kind")?.filter(|value| !value.is_empty() && value != "all");
    let source = util::optional_text(map, "sourceId")?.filter(|value| !value.trim().is_empty());
    let before = util::optional_text(map, "before")?.filter(|value| !value.trim().is_empty());
    let dry_run = matches!(map.get("dryRun"), Some(Value::Bool(true)));
    let storage_status = status.as_deref().map(|value| match value {
        "running" => "leased",
        "succeeded" => "done",
        "failed" | "cancelled" => "cancelled",
        other => other,
    });
    let filter = TaskFilter {
        status: storage_status.map(str::to_string),
        source: source.clone(),
        before: before.clone(),
        ..TaskFilter::default()
    };
    let matched_store = state
        .services
        .queue_count(&filter)
        .await
        .map_err(ApiError::from)?;
    let control = control_tasks(
        &state,
        status.as_deref(),
        kind.as_deref(),
        source.as_deref(),
        before.as_deref(),
    )
    .await?;
    if dry_run {
        return Ok(util::ok(&json!({
            "matched": matched_store + control.len() as u64,
            "deleted": 0,
            "dryRun": true,
        })));
    }
    // 活动任务先协作取消，再由存储层物理删除。
    for active in ["queued", "leased"] {
        if storage_status.is_some() && storage_status != Some(active) {
            continue;
        }
        let active_filter = TaskFilter {
            status: Some(active.to_string()),
            source: source.clone(),
            before: before.clone(),
            ..TaskFilter::default()
        };
        let _ = cancel_matching(&state, &active_filter).await?;
    }
    let purge_filter = TaskFilter {
        status: storage_status.map(|value| {
            if matches!(value, "queued" | "leased") {
                "cancelled".to_string()
            } else {
                value.to_string()
            }
        }),
        source: source.clone(),
        before: before.clone(),
        ..TaskFilter::default()
    };
    let deleted_store = state
        .services
        .queue_purge(&purge_filter)
        .await
        .map_err(ApiError::from)?;
    let mut deleted_control = 0u64;
    for doc in &control {
        if state.store.delete_document("task", &doc.id).await? {
            deleted_control += 1;
        }
    }
    crate::observe::append_audit(
        &state,
        "tasks.purge",
        "succeeded",
        "清空任务",
        json!({
            "status": status,
            "kind": kind,
            "sourceId": source,
            "before": before,
            "deleted": deleted_store + deleted_control,
        }),
        None,
    )
    .await?;
    Ok(util::ok(&json!({
        "matched": matched_store + control.len() as u64,
        "deleted": deleted_store + deleted_control,
        "dryRun": false,
    })))
}

async fn cancel_matching(state: &AppState, filter: &TaskFilter) -> Result<u64, ApiError> {
    let mut cancelled = 0u64;
    let mut cursor: Option<String> = None;
    loop {
        let page = state
            .services
            .queue_list(filter, cursor.as_deref(), 500)
            .await
            .map_err(ApiError::from)?;
        for row in &page.rows {
            if let Ok(task_id) = strata_model::TaskId::new(&row.task_id)
                && state.services.queue_cancel(&task_id).await.unwrap_or(false)
            {
                cancelled += 1;
            }
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    Ok(cancelled)
}

async fn control_tasks(
    state: &AppState,
    status: Option<&str>,
    kind: Option<&str>,
    source: Option<&str>,
    before: Option<&str>,
) -> Result<Vec<crate::store::Document>, ApiError> {
    let mut out = Vec::new();
    for doc in state.store.list_documents("task").await? {
        if let Some(status) = status
            && doc.body["status"] != status
        {
            continue;
        }
        if let Some(kind) = kind
            && doc.body["kind"] != kind
        {
            continue;
        }
        if let Some(source) = source
            && doc.body["sourceId"] != source
        {
            continue;
        }
        if let Some(before) = before
            && doc.body["createdAt"].as_str().unwrap_or("") >= before
        {
            continue;
        }
        out.push(doc);
    }
    Ok(out)
}

async fn list_queues(State(state): State<Shared>) -> Result<Response, ApiError> {
    let states = state
        .services
        .queue_states()
        .await
        .map_err(ApiError::from)?;
    let items: Vec<Value> = states
        .into_iter()
        .map(|queue| {
            // 排队却派发不动的可解释原因：暂停优先，其次并发上限已被占满。
            let stalled =
                queue.queued > 0 && (queue.paused || (queue.leased as usize) >= queue.capacity);
            let reason = if queue.paused && queue.queued > 0 {
                "队列已暂停"
            } else if stalled {
                "并发已满"
            } else {
                ""
            };
            json!({
                "id": queue.queue,
                "queued": queue.queued,
                "running": queue.leased,
                "done": queue.done,
                "cancelled": queue.cancelled,
                "capacity": queue.capacity,
                "paused": queue.paused,
                "stalled": stalled,
                "reason": reason,
            })
        })
        .collect();
    Ok(util::ok(&page(items, None)))
}

async fn pause_queue(
    State(state): State<Shared>,
    Path(queue_id): Path<String>,
) -> Result<Response, ApiError> {
    set_queue_paused(&state, &queue_id, true).await
}

async fn resume_queue(
    State(state): State<Shared>,
    Path(queue_id): Path<String>,
) -> Result<Response, ApiError> {
    set_queue_paused(&state, &queue_id, false).await
}

/// 队列级暂停/恢复：能力早已存在（`Services::queue_pause`），这里补上控制台入口。
async fn set_queue_paused(
    state: &Shared,
    queue_id: &str,
    paused: bool,
) -> Result<Response, ApiError> {
    let queue = match queue_id {
        "free" => strata_model::QueueName::Free,
        "paid" => strata_model::QueueName::Paid,
        "render" => strata_model::QueueName::Render,
        "push" => strata_model::QueueName::Push,
        other => {
            return Err(ApiError::validation(
                "queueId",
                "enum",
                format!("未知队列: {other}"),
            ));
        }
    };
    state
        .services
        .queue_pause(queue, paused)
        .await
        .map_err(ApiError::from)?;
    Ok(util::ok(&json!({ "id": queue_id, "paused": paused })))
}

/// 立即执行一轮声明式派生：把 30s 自动循环的能力补上手动入口（与采集同构）。
async fn run_derivations(State(state): State<Shared>) -> Result<Response, ApiError> {
    let operation = start(
        &state,
        "derivation.run",
        vec![json!({"kind": "derivationRun", "id": new_id("drun")})],
        json!({"kind": "derivation.run"}),
    )
    .await?;
    Ok(accepted(&operation))
}

async fn execute_derivation_run(
    state: &AppState,
    _operation_id: &str,
    _action: &Value,
) -> Result<Value, ApiError> {
    let run = state
        .services
        .run_derivations()
        .await
        .map_err(ApiError::from)?;
    Ok(json!({
        "partial": false,
        "leased": run.leased,
        "produced": run.produced,
        "skipped": run.skipped,
        "failed": run.failed,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_failure_without_task_reports_reason() {
        let notice = collection_notice(
            true,
            "",
            "WIRED",
            "该信源没有可用的采集入口，请先补齐访问器或用推送端提交",
            Some("sources?query=wired".into()),
        );
        assert_eq!(notice.event, "collection.failed");
        assert_eq!(notice.title, "采集失败：WIRED");
        assert!(notice.message.contains("采集入口"));
    }

    #[test]
    fn collection_failure_without_source_still_names_the_event() {
        let notice = collection_notice(true, "", "", "", None);
        assert_eq!(notice.title, "采集失败");
        assert_eq!(notice.message, "采集未能开始");
    }

    #[test]
    fn collection_notice_follows_task_status() {
        let done = collection_notice(false, "done", "WIRED", "", None);
        assert_eq!(done.event, "collection.completed");
        let queued = collection_notice(false, "queued", "WIRED", "", None);
        assert_eq!(queued.event, "collection.started");
        let task_failed = collection_notice(true, "failed", "WIRED", "超时", None);
        assert_eq!(task_failed.event, "collection.failed");
        assert_eq!(task_failed.message, "超时");
        let silent = collection_notice(true, "failed", "WIRED", "", None);
        assert_eq!(silent.message, "任务以失败结束");
    }
}
