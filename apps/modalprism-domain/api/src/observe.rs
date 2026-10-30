//! 三类观测流：日志、结构化事件与审计。审计只追加，不随普通日志清理。

use axum::Json;
use axum::Router;
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::atomic::Ordering;
use strata_client::TaskFilter;

use crate::error::ApiError;
use crate::operations::accepted;
use crate::util::{self, cursor, page_total};
use crate::{AppState, Shared, new_id, now};

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/observability/logs", get(list_logs))
        .route("/observability/events", get(list_events))
        .route("/observability/auditEvents", get(list_audit))
        .route("/observability/exports", post(create_export))
}

/// 每小时最多执行一次保留策略，避免每次请求都扫表。
pub async fn maybe_retention(state: &AppState) {
    if state.in_maintenance() {
        return;
    }
    let now = time::OffsetDateTime::now_utc().unix_timestamp().max(0) as u64;
    let last = state.retention_at.load(Ordering::Relaxed);
    if now.saturating_sub(last) < 3_600 {
        return;
    }
    if state
        .retention_at
        .compare_exchange(last, now, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    let _ = run_retention(state).await;
}

async fn run_retention(state: &AppState) -> Result<(), ApiError> {
    let now = time::OffsetDateTime::now_utc();
    for (stream, key, fallback) in [
        ("logs", "retention.logDays", 14i64),
        ("events", "retention.eventDays", 30),
        ("audit", "retention.auditDays", 365),
    ] {
        let days = crate::settings::value(state, key)
            .await?
            .as_i64()
            .unwrap_or(fallback)
            .max(1);
        let cutoff = (now - time::Duration::days(days))
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();
        state.store.prune_records_before(stream, &cutoff).await?;
    }
    let task_days = crate::settings::value(state, "collection.taskRetentionDays")
        .await?
        .as_i64()
        .unwrap_or(7)
        .clamp(1, 365);
    let task_cutoff = (now - time::Duration::days(task_days))
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    for status in ["done", "cancelled"] {
        state
            .services
            .queue_purge(&TaskFilter {
                status: Some(status.to_string()),
                before: Some(task_cutoff.clone()),
                ..TaskFilter::default()
            })
            .await
            .map_err(ApiError::from)?;
    }
    let keep = crate::settings::value(state, "retention.healthHistoryLimit")
        .await?
        .as_u64()
        .unwrap_or(500)
        .clamp(10, 5_000) as usize;
    state.store.trim_documents("health_history", keep).await?;
    Ok(())
}

/// 关联字段集合。无关字段保持 null，不省略键。
#[derive(Clone, Debug, Default)]
pub struct Refs {
    pub operation_id: Option<String>,
    pub task_id: Option<String>,
    pub source_id: Option<String>,
    pub content_id: Option<String>,
    pub run_id: Option<String>,
    pub attempt_id: Option<String>,
    pub delivery_id: Option<String>,
}

impl Refs {
    pub fn op(id: &str) -> Self {
        Self {
            operation_id: Some(id.to_string()),
            ..Self::default()
        }
    }

    pub fn content(id: &str) -> Self {
        Self {
            content_id: Some(id.to_string()),
            ..Self::default()
        }
    }
}

pub async fn append(
    state: &AppState,
    stream: &str,
    level: &str,
    kind: &str,
    message: &str,
    data: Value,
    refs: Refs,
) -> Result<(), ApiError> {
    let record = json!({
        "timestamp": now(),
        "level": level,
        "kind": kind,
        "message": message,
        "requestId": crate::error::request_id_opt(),
        "operationId": refs.operation_id,
        "taskId": refs.task_id,
        "sourceId": refs.source_id,
        "contentId": refs.content_id,
        "runId": refs.run_id,
        "attemptId": refs.attempt_id,
        "deliveryId": refs.delivery_id,
        "configGeneration": state.services.config_generation() as i64,
        "data": data,
    });
    state.store.append_record(&record, stream).await?;
    Ok(())
}

pub async fn append_log(
    state: &AppState,
    level: &str,
    kind: &str,
    message: &str,
    data: Value,
    operation_id: Option<&str>,
) -> Result<(), ApiError> {
    append(
        state,
        "logs",
        level,
        kind,
        message,
        data,
        Refs {
            operation_id: operation_id.map(str::to_string),
            ..Refs::default()
        },
    )
    .await
}

pub async fn append_event(
    state: &AppState,
    kind: &str,
    message: &str,
    data: Value,
    refs: Refs,
) -> Result<(), ApiError> {
    append(state, "events", "info", kind, message, data, refs).await
}

pub async fn append_audit(
    state: &AppState,
    kind: &str,
    result: &str,
    message: &str,
    data: Value,
    refs: Option<Refs>,
) -> Result<(), ApiError> {
    let mut payload = data;
    if let Some(map) = payload.as_object_mut() {
        map.insert("result".into(), Value::String(result.into()));
    }
    append(
        state,
        "audit",
        if result == "failed" { "warn" } else { "info" },
        kind,
        message,
        payload,
        refs.unwrap_or_default(),
    )
    .await
}

/// 运行时守护：轮询调度器状态与任务终态，把状态跃迁补记为追加式审计与通知。
///
/// 覆盖四类关键事件：计划命中停止/恢复、信源健康降级/恢复、新的失败任务、当日产出增量。
/// 只在跃迁时触发，稳态零噪声；首次轮询只建基线，不补发历史，避免重启后刷屏。
/// 不改动派发/去重链路（保持 AccessTask/队列模型不变）。
pub async fn watch_plan_stops(state: &Shared) {
    use std::collections::{BTreeMap, HashSet};
    let mut last: BTreeMap<String, String> = BTreeMap::new();
    let mut health: BTreeMap<String, String> = BTreeMap::new();
    let mut yields: BTreeMap<String, u64> = BTreeMap::new();
    let mut seen_tasks: HashSet<String> = HashSet::new();
    let mut primed = false;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(45)).await;
        if state.in_maintenance() || state.reset_in_progress() {
            continue;
        }
        let Ok(statuses) = state.services.planner_statuses().await else {
            continue;
        };
        let names = source_names(state).await;
        let mut current: BTreeMap<String, String> = BTreeMap::new();
        let mut current_health: BTreeMap<String, String> = BTreeMap::new();
        let mut current_yields: BTreeMap<String, u64> = BTreeMap::new();
        let mut produced: Vec<(String, u64)> = Vec::new();
        for (source, status) in &statuses {
            let label = source_label(&names, source);
            if let Some(reason) = status.skip_reason.as_deref()
                && matches!(
                    reason,
                    "quota-paused" | "quota-exhausted" | "budget" | "daily-cap"
                )
            {
                if last.get(source).map(String::as_str) != Some(reason) {
                    let _ = append_audit(
                        state,
                        "collection.plan.stop",
                        "succeeded",
                        "采集计划命中停止策略，暂停派发",
                        json!({"sourceId": source, "reason": reason}),
                        Some(Refs {
                            source_id: Some(source.clone()),
                            ..Refs::default()
                        }),
                    )
                    .await;
                    if primed {
                        crate::notifications::emit(
                            state,
                            "plan",
                            "plan.stopped",
                            "warn",
                            format!("采集计划暂停：{label}"),
                            format!("原因：{}", stop_label(reason)),
                            Some(&format!("plans?target={source}")),
                        )
                        .await;
                    }
                }
                current.insert(source.clone(), reason.to_string());
            }
            let current_health_value = status.health_status.clone();
            if primed
                && let Some(previous) = health.get(source)
                && previous != &current_health_value
            {
                if health_bad(&current_health_value) {
                    crate::notifications::emit(
                        state,
                        "source",
                        "source.unhealthy",
                        "warn",
                        format!("信源异常：{label}"),
                        status.last_error.clone().unwrap_or_else(|| {
                            format!("健康状态：{}", health_label(&current_health_value))
                        }),
                        Some(&format!(
                            "sources?query={}",
                            source.trim_start_matches("source/")
                        )),
                    )
                    .await;
                } else if health_bad(previous)
                    && matches!(current_health_value.as_str(), "ok" | "healthy")
                {
                    crate::notifications::emit(
                        state,
                        "source",
                        "source.recovered",
                        "info",
                        format!("信源恢复：{label}"),
                        "健康状态已回到正常范围".to_string(),
                        Some(&format!(
                            "sources?query={}",
                            source.trim_start_matches("source/")
                        )),
                    )
                    .await;
                }
            }
            current_health.insert(source.clone(), current_health_value);
            if primed
                && let Some(previous) = yields.get(source)
                && status.today_yield > *previous
            {
                produced.push((label, status.today_yield - *previous));
            }
            current_yields.insert(source.clone(), status.today_yield);
        }
        if primed {
            if !produced.is_empty() {
                let total: u64 = produced.iter().map(|(_, count)| *count).sum();
                let detail = produced
                    .iter()
                    .take(3)
                    .map(|(label, count)| format!("{label} +{count}"))
                    .collect::<Vec<_>>()
                    .join("，");
                let suffix = if produced.len() > 3 { " 等" } else { "" };
                crate::notifications::emit(
                    state,
                    "content",
                    "content.produced",
                    "info",
                    "内容入库",
                    format!("{total} 条新内容：{detail}{suffix}"),
                    Some("content"),
                )
                .await;
            }
            for (source, previous) in &last {
                if !current.contains_key(source) {
                    let _ = append_audit(
                        state,
                        "collection.plan.resume",
                        "succeeded",
                        "采集计划恢复派发",
                        json!({"sourceId": source, "previousReason": previous}),
                        Some(Refs {
                            source_id: Some(source.clone()),
                            ..Refs::default()
                        }),
                    )
                    .await;
                    crate::notifications::emit(
                        state,
                        "plan",
                        "plan.resumed",
                        "info",
                        format!("采集计划恢复：{}", source_label(&names, source)),
                        "停止条件已解除，恢复派发".to_string(),
                        Some(&format!("plans?target={source}")),
                    )
                    .await;
                }
            }
        }
        last = current;
        health = current_health;
        yields = current_yields;
        // 失败任务：按任务号去重，只报本次新增的终态失败。
        if let Ok(page) = state
            .services
            .queue_list(
                &TaskFilter {
                    status: Some("failed".to_string()),
                    ..TaskFilter::default()
                },
                None,
                200,
            )
            .await
        {
            let mut window = HashSet::new();
            for row in &page.rows {
                window.insert(row.task_id.clone());
                if !primed || seen_tasks.contains(&row.task_id) {
                    continue;
                }
                crate::notifications::emit(
                    state,
                    "source",
                    "collection.failed",
                    "warn",
                    format!("采集失败：{}", source_label(&names, &row.source_id)),
                    row.last_error
                        .clone()
                        .unwrap_or_else(|| "任务以失败结束".to_string()),
                    Some("tasks?status=failed"),
                )
                .await;
            }
            seen_tasks.extend(window.clone());
            if seen_tasks.len() > 4_000 {
                seen_tasks = window;
            }
        }
        primed = true;
    }
}

fn health_bad(status: &str) -> bool {
    matches!(
        status,
        "degraded" | "failed" | "suspicious" | "misconfigured" | "configurationError"
    )
}

fn health_label(status: &str) -> &str {
    match status {
        "ok" | "healthy" => "正常",
        "degraded" => "能力受限",
        "failed" => "失败",
        "suspicious" => "疑似异常",
        "misconfigured" | "configurationError" => "配置错误",
        "paused" => "暂停",
        other => other,
    }
}

fn stop_label(reason: &str) -> &str {
    match reason {
        "quota-paused" => "额度暂停",
        "quota-exhausted" => "额度耗尽",
        "budget" => "预算用尽",
        "daily-cap" => "达到每日上限",
        other => other,
    }
}

fn source_label(names: &std::collections::BTreeMap<String, String>, source: &str) -> String {
    let slug = source.trim_start_matches("source/");
    names.get(slug).cloned().unwrap_or_else(|| slug.to_string())
}

async fn source_names(state: &AppState) -> std::collections::BTreeMap<String, String> {
    let mut names = std::collections::BTreeMap::new();
    let Ok(list) = config::list(&state.config_dir) else {
        return names;
    };
    for source in list["sources"].as_array().cloned().unwrap_or_default() {
        let Some(slug) = source["slug"].as_str() else {
            continue;
        };
        names.insert(
            slug.to_string(),
            source["name"].as_str().unwrap_or(slug).to_string(),
        );
    }
    names
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
    level: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    correlation_id: Option<String>,
}

async fn list_logs(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    list_stream(&state, "logs", query).await
}

async fn list_events(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    list_stream(&state, "events", query).await
}

async fn list_audit(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    list_stream(&state, "audit", query).await
}

async fn list_stream(
    state: &AppState,
    stream: &str,
    query: ListQuery,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let filter = format!(
        "{stream}:{}:{}:{}:{}",
        query.level.clone().unwrap_or_default(),
        query.kind.clone().unwrap_or_default(),
        query.correlation_id.clone().unwrap_or_default(),
        query.query.clone().unwrap_or_default()
    );
    let offset = util::page_offset(query.offset.as_deref(), query.cursor.as_deref(), &filter)?;
    // 游标是筛选快照内的偏移：先取 offset+limit+1 行，再截取，避免客户端解析 seq。
    let record_filter = crate::store::RecordFilter {
        level: query.level.as_deref().filter(|value| !value.is_empty()),
        kind: query.kind.as_deref().filter(|value| !value.is_empty()),
        correlation: query
            .correlation_id
            .as_deref()
            .filter(|value| !value.is_empty()),
        query: query.query.as_deref().filter(|value| !value.is_empty()),
    };
    let rows = state
        .store
        .list_records(stream, offset + limit + 1, record_filter)
        .await?;
    let window = rows.len();
    let items: Vec<Value> = rows
        .into_iter()
        .skip(offset)
        .take(limit + 1)
        .map(|(seq, mut value)| {
            if let Some(map) = value.as_object_mut() {
                map.insert("cursor".into(), Value::String(seq.to_string()));
                map.insert("stream".into(), Value::String(stream.into()));
            }
            value
        })
        .collect();
    let more = items.len() > limit;
    let items: Vec<Value> = items.into_iter().take(limit).collect();
    let next = if more || offset + items.len() < window {
        Some(cursor(&filter, offset + limit))
    } else {
        None
    };
    let total = state
        .store
        .count_records_filtered(stream, record_filter)
        .await?;
    Ok(util::ok(&page_total(items, next, total)))
}

async fn create_export(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["stream", "format", "filters"])?;
    let stream = util::text(map, "stream")?;
    if !matches!(stream.as_str(), "logs" | "events" | "audit") {
        return Err(ApiError::validation(
            "stream",
            "enum",
            "只支持 logs、events 或 audit",
        ));
    }
    let format = util::text(map, "format")?;
    if !matches!(format.as_str(), "jsonl" | "csv") {
        return Err(ApiError::validation(
            "format",
            "enum",
            "只支持 jsonl 或 csv",
        ));
    }
    let filters = util::optional_object(map, "filters")?.unwrap_or_default();
    let operation = crate::operations::start(
        &state,
        "observability.export",
        vec![json!({"kind": "export", "id": new_id("export")})],
        json!({"kind": "logs.export", "stream": stream, "format": format, "filters": filters}),
    )
    .await?;
    Ok(accepted(&operation))
}

pub async fn execute_export(
    state: &AppState,
    operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let stream = action["stream"].as_str().unwrap_or("logs");
    let format = action["format"].as_str().unwrap_or("jsonl");
    let filters = action.get("filters").cloned().unwrap_or_else(|| json!({}));
    let rows = state
        .store
        .list_records(
            stream,
            5000,
            crate::store::RecordFilter {
                level: filters.get("level").and_then(Value::as_str),
                kind: filters.get("kind").and_then(Value::as_str),
                correlation: filters.get("correlationId").and_then(Value::as_str),
                query: filters.get("query").and_then(Value::as_str),
            },
        )
        .await?;
    let content = if format == "csv" {
        let mut out = String::from("timestamp,level,kind,message\n");
        for (_, item) in &rows {
            out.push_str(&format!(
                "{},{},{},{}\n",
                csv(item["timestamp"].as_str().unwrap_or("")),
                csv(item["level"].as_str().unwrap_or("")),
                csv(item["kind"].as_str().unwrap_or("")),
                csv(item["message"].as_str().unwrap_or(""))
            ));
        }
        out
    } else {
        rows.iter()
            .map(|(_, item)| item.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let export_id = new_id("export");
    let ttl_hours = crate::settings::value(state, "storage.exportTtlHours")
        .await?
        .as_i64()
        .unwrap_or(24)
        .clamp(1, 168);
    let doc = json!({
        "id": export_id,
        "stream": stream,
        "format": format,
        "count": rows.len(),
        "content": content,
        "createdAt": now(),
        "expiresAt": time::OffsetDateTime::now_utc()
            .checked_add(time::Duration::hours(ttl_hours))
            .and_then(|at| at.format(&time::format_description::well_known::Rfc3339).ok())
            .unwrap_or_else(now),
    });
    state
        .store
        .insert_document("export", &export_id, &doc)
        .await?;
    append_audit(
        state,
        "observability.export",
        "succeeded",
        "导出观测流",
        json!({"stream": stream, "format": format, "count": rows.len()}),
        Some(Refs::op(operation_id)),
    )
    .await?;
    Ok(json!({
        "partial": false,
        "exportId": export_id,
        "stream": stream,
        "format": format,
        "count": rows.len(),
        "content": doc["content"],
        "expiresAt": doc["expiresAt"],
    }))
}

fn csv(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}
