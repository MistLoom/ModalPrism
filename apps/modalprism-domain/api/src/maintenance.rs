//! 数据清空与恢复出厂：预演、确认令牌、维护模式与完整性报告。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use axum::Json;
use axum::Router;
use axum::extract::{Path as AxPath, State};
use axum::http::{HeaderMap, header};
use axum::response::Response;
use axum::routing::{get, post};
use serde_json::{Value, json};
use strata_client::TaskFilter;

use crate::error::ApiError;
use crate::operations::accepted;
use crate::util;
use crate::{AppState, Shared, new_id, now};

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/maintenance/resetPreviews", post(reset_preview))
        .route("/maintenance/resets", post(create_reset))
        .route("/maintenance/factoryResetPreviews", post(factory_preview))
        .route("/maintenance/factoryResets", post(create_factory_reset))
        .route(
            "/maintenance/integrityReports/{reportId}",
            get(integrity_report),
        )
}

const DOMAINS: &[&str] = &[
    "tasks",
    "content",
    "blobs",
    "derivedData",
    "delivery",
    "staging",
    "logs",
    "audit",
    "operations",
    "userSettings",
];

async fn count_docs(state: &AppState, kinds: &[&str]) -> Result<u64, ApiError> {
    let mut total = 0;
    for kind in kinds {
        total += state.store.count_kind(kind).await?;
    }
    Ok(total)
}

async fn dir_stats(path: &Path) -> (u64, u64) {
    let mut files = 0u64;
    let mut bytes = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(mut entries) = tokio::fs::read_dir(&current).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let Ok(meta) = entry.metadata().await else {
                continue;
            };
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                files += 1;
                bytes += meta.len();
            }
        }
    }
    (files, bytes)
}

async fn preview_counts(state: &AppState, domains: &[String]) -> Result<Value, ApiError> {
    let mut out = serde_json::Map::new();
    for domain in domains {
        let value = match domain.as_str() {
            "tasks" => {
                let stats = state.services.queue_stats().await.map_err(ApiError::from)?;
                let control = count_docs(state, &["task"]).await?;
                json!({"count": stats.total + control, "bytes": 0})
            }
            "content" => {
                let counts = state
                    .services
                    .store_counts()
                    .await
                    .map_err(ApiError::from)?;
                json!({"count": counts.resources, "bytes": 0})
            }
            "blobs" => {
                let report = state
                    .services
                    .strata()
                    .purge_unreferenced_blobs(true)
                    .await
                    .map_err(ApiError::from)?;
                let count = report.tables.get("g_blobs").copied().unwrap_or(0);
                json!({"count": count, "bytes": 0})
            }
            "derivedData" => {
                let summary = state
                    .services
                    .strata()
                    .summary()
                    .await
                    .map_err(ApiError::from)?;
                let derived: u64 = summary
                    .variants
                    .iter()
                    .filter(|variant| {
                        variant.variant != "article"
                            && variant.variant != "hotlist"
                            && variant.variant != "discovery"
                    })
                    .map(|variant| variant.total)
                    .sum();
                json!({"count": derived, "bytes": 0})
            }
            "delivery" => {
                let count = count_docs(
                    state,
                    &[
                        "delivery_rule",
                        "delivery_endpoint",
                        "delivery_record",
                        "notification_entry",
                        "notification_center_settings",
                        "push_channel",
                        "push_record",
                        "channel",
                        "publication",
                        "delivery",
                        "export",
                        "upload",
                        // 旧加工数据：功能已下线，仍随该域清理。
                        "input_set",
                        "workflow",
                        "workflow_version",
                        "production_run",
                        "production_attempt",
                        "candidate",
                        "review",
                        "draft",
                    ],
                )
                .await?;
                json!({"count": count, "bytes": 0})
            }
            "staging" => {
                let staging = state.data_dir.join("staging");
                let uploads = state.data_dir.join("uploads");
                let (files_a, bytes_a) = dir_stats(&staging).await;
                let (files_b, bytes_b) = dir_stats(&uploads).await;
                json!({"count": files_a + files_b, "bytes": bytes_a + bytes_b})
            }
            "logs" => json!({"count": state.store.count_records("logs").await?, "bytes": 0}),
            "audit" => json!({"count": state.store.count_records("audit").await?, "bytes": 0}),
            "operations" => {
                let finished = state.store.count_finished_operations().await?;
                json!({"count": finished, "bytes": 0})
            }
            "userSettings" => {
                let count = count_docs(
                    state,
                    &[
                        "settings",
                        "settings_history",
                        "model",
                        "prompt",
                        "skill",
                        "channel",
                    ],
                )
                .await?;
                json!({"count": count, "bytes": 0})
            }
            other => {
                return Err(ApiError::validation(
                    "domains",
                    "enum",
                    format!("未知清空域: {other}"),
                ));
            }
        };
        out.insert(domain.clone(), value);
    }
    Ok(Value::Object(out))
}

fn dependencies(domains: &[String]) -> Vec<Value> {
    let mut added = Vec::new();
    if domains.iter().any(|domain| domain == "content") {
        for domain in ["derivedData", "blobs"] {
            if !domains.iter().any(|value| value == domain) {
                added.push(json!({
                    "domain": domain,
                    "reason": "内容墓碑与派生结果、无引用对象存在依赖"
                }));
            }
        }
    }
    if domains.iter().any(|domain| domain == "delivery")
        && !domains.iter().any(|value| value == "content")
    {
        added.push(json!({
            "domain": "content",
            "reason": "投递记录保存了内容快照，清理内容会一并清空投递记录"
        }));
    }
    added
}

fn expanded(domains: &[String]) -> Vec<String> {
    let mut out: Vec<String> = domains.to_vec();
    for dep in dependencies(domains) {
        if let Some(domain) = dep["domain"].as_str()
            && !out.iter().any(|value| value == domain)
        {
            out.push(domain.to_string());
        }
    }
    out.sort();
    out.dedup();
    out
}

async fn ttl_minutes(state: &AppState) -> i64 {
    crate::settings::value(state, "maintenance.resetTokenTtlMinutes")
        .await
        .ok()
        .and_then(|value| value.as_i64())
        .unwrap_or(10)
}

fn expires_at(minutes: i64) -> String {
    time::OffsetDateTime::now_utc()
        .checked_add(time::Duration::minutes(minutes))
        .and_then(|at| {
            at.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_else(now)
}

async fn build_preview(
    state: &AppState,
    kind: &str,
    domains: &[String],
) -> Result<Value, ApiError> {
    let scopes = expanded(domains);
    let counts = preview_counts(state, &scopes).await?;
    let storage_reachable = state.services.strata().summary().await.is_ok();
    let id = new_id("preview");
    let token = format!("confirm_{}", ulid::Ulid::new().to_string().to_lowercase());
    let expires = expires_at(ttl_minutes(state).await);
    let etag = format!("\"preview-{id}-v1\"");
    let preview = json!({
        "id": id,
        "kind": kind,
        "domains": scopes,
        "requested": domains,
        "counts": counts,
        "dependencies": dependencies(domains),
        "confirmationToken": token,
        "expiresAt": expires,
        "etag": etag.trim_matches('"'),
        "generation": state.services.config_generation(),
        "storageReachable": storage_reachable,
        "createdAt": now(),
        "irreversible": true,
        "note": "执行前请确认范围；执行期间进入维护模式。",
    });
    state
        .store
        .insert_preview(&id, kind, &preview, &token, &etag, &expires)
        .await?;
    Ok(preview)
}

async fn reset_preview(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    if state.in_maintenance() || state.reset_in_progress() {
        return Err(ApiError::conflict("已有维护操作在进行"));
    }
    let map = util::object(&body)?;
    util::unknown(map, &["domains"])?;
    let domains = util::strings(map, "domains")?;
    if domains.is_empty() {
        return Err(ApiError::validation(
            "domains",
            "required",
            "至少选择一个域",
        ));
    }
    for domain in &domains {
        if !DOMAINS.contains(&domain.as_str()) {
            return Err(ApiError::validation(
                "domains",
                "enum",
                format!("未知清空域: {domain}"),
            ));
        }
    }
    let preview = build_preview(&state, "reset", &domains).await?;
    let mut response = util::created(&preview);
    if let Ok(value) = preview["etag"].as_str().unwrap_or("").parse() {
        response.headers_mut().insert(header::ETAG, value);
    }
    Ok(response)
}

async fn factory_preview(
    State(state): State<Shared>,
    Json(_body): Json<Value>,
) -> Result<Response, ApiError> {
    if state.in_maintenance() || state.reset_in_progress() {
        return Err(ApiError::conflict("已有维护操作在进行"));
    }
    let domains: Vec<String> = DOMAINS.iter().map(|value| value.to_string()).collect();
    let preview = build_preview(&state, "factory", &domains).await?;
    let mut response = util::created(&preview);
    if let Ok(value) = preview["etag"].as_str().unwrap_or("").parse() {
        response.headers_mut().insert(header::ETAG, value);
    }
    Ok(response)
}

async fn confirm_preview(
    state: &AppState,
    headers: &HeaderMap,
    body: &Value,
    kind: &str,
) -> Result<(Value, Vec<String>), ApiError> {
    let map = util::object(body)?;
    util::unknown(map, &["previewId", "confirmationToken"])?;
    let preview_id = util::non_empty(map, "previewId")?;
    let token = util::non_empty(map, "confirmationToken")?;
    let preview = state
        .store
        .get_preview(&preview_id)
        .await?
        .ok_or_else(|| ApiError::precondition("预演不存在或已过期，请重新预演"))?;
    if preview["kind"] != kind {
        return Err(ApiError::precondition("预演类型不匹配"));
    }
    if preview["confirmationToken"] != token {
        return Err(ApiError::precondition("确认令牌无效"));
    }
    let expires = preview["expiresAt"].as_str().unwrap_or_default();
    let expires_at =
        time::OffsetDateTime::parse(expires, &time::format_description::well_known::Rfc3339)
            .map_err(|_| ApiError::precondition("确认令牌过期"))?;
    if expires_at < time::OffsetDateTime::now_utc() {
        return Err(ApiError::precondition("确认令牌已过期，请重新预演"));
    }
    util::if_match(headers, preview["etag"].as_str().unwrap_or_default())?;
    if !preview["storageReachable"].as_bool().unwrap_or(false) {
        return Err(ApiError::dependency("存储不可达，请恢复后重新预演"));
    }
    let domains: Vec<String> = preview["domains"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect();
    if preview["generation"].as_i64() != Some(state.services.config_generation() as i64) {
        return Err(ApiError::precondition("配置代际已变化，请重新预演"));
    }
    let preview_counts_value = preview["counts"].clone();
    // 日志/审计/操作/任务流在预览与确认之间会因正常请求增长，无法作为冻结数量；
    // 其余域必须与预演完全一致。
    let stable: Vec<String> = domains
        .iter()
        .filter(|domain| {
            !matches!(
                domain.as_str(),
                "logs" | "audit" | "operations" | "tasks" | "staging"
            )
        })
        .cloned()
        .collect();
    if !stable.is_empty() {
        let actual = preview_counts(state, &stable).await?;
        for domain in &stable {
            if actual[domain] != preview_counts_value[domain] {
                return Err(ApiError::precondition(format!(
                    "域 {domain} 的数量已变化，请重新预演"
                )));
            }
        }
    }
    Ok((preview, domains))
}

async fn create_reset(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let (preview, domains) = confirm_preview(&state, &headers, &body, "reset").await?;
    if !state.try_begin_reset() {
        return Err(ApiError::conflict("已有维护操作在进行"));
    }
    let operation = match crate::operations::start(
        &state,
        "maintenance.reset",
        vec![json!({"kind": "reset", "id": preview["id"]})],
        json!({
            "kind": "maintenance.reset",
            "previewId": preview["id"],
            "domains": domains,
            "counts": preview["counts"],
        }),
    )
    .await
    {
        Ok(operation) => operation,
        Err(error) => {
            state.end_reset();
            return Err(error);
        }
    };
    state
        .store
        .delete_preview(preview["id"].as_str().unwrap_or_default())
        .await?;
    Ok(accepted(&operation))
}

async fn create_factory_reset(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let (preview, domains) = confirm_preview(&state, &headers, &body, "factory").await?;
    if !state.try_begin_reset() {
        return Err(ApiError::conflict("已有维护操作在进行"));
    }
    let operation = match crate::operations::start(
        &state,
        "maintenance.factoryReset",
        vec![json!({"kind": "factoryReset", "id": preview["id"]})],
        json!({
            "kind": "maintenance.factoryReset",
            "previewId": preview["id"],
            "domains": domains,
            "counts": preview["counts"],
        }),
    )
    .await
    {
        Ok(operation) => operation,
        Err(error) => {
            state.end_reset();
            return Err(error);
        }
    };
    state
        .store
        .delete_preview(preview["id"].as_str().unwrap_or_default())
        .await?;
    Ok(accepted(&operation))
}

async fn integrity_report(
    State(state): State<Shared>,
    AxPath(report_id): AxPath<String>,
) -> Result<Response, ApiError> {
    let doc = state
        .store
        .get_document("integrity_report", &report_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("完整性报告 {report_id}")))?;
    Ok(util::ok(&doc.body))
}

pub async fn execute_reset(
    state: &AppState,
    operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let domains: Vec<String> = action["domains"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect();
    let preview_counts = action["counts"].clone();
    state.enter_maintenance();
    let result = run_reset(state, operation_id, &domains, &preview_counts, false).await;
    // 失败时保持维护模式，避免残留数据继续被写入；重启或成功重试后退出。
    if result.is_ok() {
        state.leave_maintenance();
    }
    state.end_reset();
    result
}

pub async fn execute_factory_reset(
    state: &AppState,
    operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let domains: Vec<String> = action["domains"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect();
    let preview_counts = action["counts"].clone();
    state.enter_maintenance();
    let result = async {
        ensure_factory_snapshot(state).await?;
        run_reset(state, operation_id, &domains, &preview_counts, true).await
    }
    .await;
    if result.is_ok() {
        state.leave_maintenance();
    }
    state.end_reset();
    result
}

async fn run_reset(
    state: &AppState,
    operation_id: &str,
    domains: &[String],
    expected_counts: &Value,
    factory: bool,
) -> Result<Value, ApiError> {
    let mut report = json!({
        "operationId": operation_id,
        "factory": factory,
        "domains": [],
        "previewCounts": expected_counts,
        "startedAt": now(),
        "cancelledTasks": 0,
        "danglingReferences": 0,
        "rebuilt": [],
        "envPreserved": std::env::vars().map(|(key, _)| key).collect::<Vec<_>>(),
    });
    let mut succeeded = Vec::new();
    let mut failed = Vec::new();
    let mut cancelled = false;
    let drain_ms = crate::settings::value(state, "maintenance.drainTimeoutMs")
        .await?
        .as_u64()
        .unwrap_or(30_000)
        .clamp(1_000, 600_000);
    let mut cancelled_tasks = 0u64;
    let started = Instant::now();
    loop {
        let stats = state.services.queue_stats().await.map_err(ApiError::from)?;
        let active = stats.by_status.get("queued").copied().unwrap_or(0)
            + stats.by_status.get("leased").copied().unwrap_or(0);
        if active == 0 {
            break;
        }
        if started.elapsed() >= Duration::from_millis(drain_ms) {
            for status in ["queued", "leased"] {
                cancelled_tasks += cancel_tasks(state, status).await?;
            }
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    report["cancelledTasks"] = json!(cancelled_tasks);
    for domain in domains {
        if crate::operations::check_cancel(state, operation_id)
            .await
            .is_err()
        {
            cancelled = true;
            break;
        }
        if factory && domain == "operations" {
            continue;
        }
        let outcome = reset_domain(state, domain).await;
        match outcome {
            Ok(counts) => {
                succeeded.push(domain.clone());
                if let Some(list) = report["domains"].as_array_mut() {
                    list.push(json!({"domain": domain, "status": "succeeded", "counts": counts}));
                }
            }
            Err(error) => {
                failed
                    .push(json!({"domain": domain, "code": error.code, "message": error.message}));
                if let Some(list) = report["domains"].as_array_mut() {
                    list.push(
                        json!({"domain": domain, "status": "failed", "message": error.message}),
                    );
                }
            }
        }
    }
    if factory {
        state
            .store
            .reset_documents_keep_operation(operation_id)
            .await?;
        restore_factory_config(state).await?;
        state.services.config_reload()?;
        crate::settings::seed(state).await?;
        crate::observe::append_audit(
            state,
            "maintenance.factoryReset",
            "succeeded",
            "恢复出厂设置",
            json!({"operationId": operation_id}),
            Some(crate::observe::Refs::op(operation_id)),
        )
        .await?;
    }
    report["succeeded"] = json!(succeeded);
    report["failed"] = json!(failed);
    report["cancelled"] = json!(cancelled);
    let actual = preview_counts(state, domains)
        .await
        .unwrap_or_else(|_| json!({}));
    report["actualCounts"] = actual.clone();
    // 业务数据必须真正归零；日志、审计与设置会被结果事件或内置资源重建。
    let checked: Vec<String> = domains
        .iter()
        .filter(|domain| {
            matches!(
                domain.as_str(),
                "tasks" | "content" | "delivery" | "staging" | "blobs"
            )
        })
        .cloned()
        .collect();
    let mut residual = serde_json::Map::new();
    for domain in &checked {
        let count = actual[domain]["count"].as_u64().unwrap_or(0);
        residual.insert(domain.clone(), json!(count));
    }
    let residual_clear = residual.values().all(|value| value.as_u64() == Some(0));
    report["residual"] = Value::Object(residual);
    report["finishedAt"] = json!(now());
    let staging_files = dir_stats(&state.data_dir.join("staging")).await.0;
    let staging_clean = !domains.iter().any(|domain| domain == "staging") || staging_files == 0;
    let indexes_consistent = state.services.strata().summary().await.is_ok();
    let builtins_ready = config::list(&state.config_dir).is_ok();
    let loopback_verified = state.http_addr.ip().is_loopback();
    report["integrity"] = json!({
        "danglingReferences": report["failed"].as_array().map_or(0, Vec::len),
        "stagingClean": staging_clean,
        "indexesConsistent": indexes_consistent,
        "loopbackVerified": loopback_verified,
        "builtinsReady": builtins_ready,
        "residualClear": residual_clear,
    });
    let integrity_ok = failed.is_empty()
        && staging_clean
        && indexes_consistent
        && loopback_verified
        && builtins_ready
        && residual_clear;
    report["integrityOk"] = json!(integrity_ok);
    let report_id = new_id("integrity");
    report["id"] = json!(report_id);
    state
        .store
        .insert_document("integrity_report", &report_id, &report)
        .await?;
    crate::observe::append_event(
        state,
        "maintenance.reset",
        if cancelled {
            "清理已取消"
        } else if failed.is_empty() {
            "清理完成"
        } else {
            "清理部分失败"
        },
        report.clone(),
        crate::observe::Refs::op(operation_id),
    )
    .await?;
    if !failed.is_empty() || !integrity_ok {
        return Err(ApiError::dependency("清理或完整性检查失败，详见完整性报告"));
    }
    Ok(json!({
        "partial": false,
        "reportId": report_id,
        "domains": report["domains"],
        "integrity": report["integrity"],
        "factory": factory,
        "cancelled": cancelled,
    }))
}

async fn cancel_tasks(state: &AppState, status: &str) -> Result<u64, ApiError> {
    let mut cancelled = 0u64;
    let mut cursor: Option<String> = None;
    loop {
        let filter = TaskFilter {
            status: Some(status.to_string()),
            ..TaskFilter::default()
        };
        let page = state
            .services
            .queue_list(&filter, cursor.as_deref(), 500)
            .await
            .map_err(ApiError::from)?;
        for row in &page.rows {
            if let Ok(task_id) = strata_model::TaskId::new(&row.task_id)
                && state
                    .services
                    .queue_cancel(&task_id)
                    .await
                    .map_err(ApiError::from)?
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

async fn reset_domain(state: &AppState, domain: &str) -> Result<Value, ApiError> {
    match domain {
        "tasks" => {
            let stats = state.services.queue_stats().await.map_err(ApiError::from)?;
            // 先协作取消所有活动任务，再物理删除全部任务行。
            let mut cancelled = 0u64;
            for status in ["queued", "leased"] {
                cancelled += cancel_tasks(state, status).await?;
            }
            let purged = state
                .services
                .queue_purge(&TaskFilter::default())
                .await
                .map_err(ApiError::from)?;
            let control = state.store.delete_kind("task").await?;
            Ok(json!({
                "total": stats.total,
                "cancelled": cancelled,
                "purged": purged,
                "control": control
            }))
        }
        "content" => {
            let report = state
                .services
                .strata()
                .purge_content()
                .await
                .map_err(ApiError::from)?;
            Ok(serde_json::to_value(report).unwrap_or_else(|_| json!({})))
        }
        "blobs" | "derivedData" | "staging" => {
            if domain == "derivedData" {
                let report = state
                    .services
                    .strata()
                    .purge_derived()
                    .await
                    .map_err(ApiError::from)?;
                return Ok(serde_json::to_value(report).unwrap_or_else(|_| json!({})));
            }
            if domain == "staging" {
                let staging = state.data_dir.join("staging");
                let uploads = state.data_dir.join("uploads");
                let (files_a, bytes_a) = dir_stats(&staging).await;
                let (files_b, bytes_b) = dir_stats(&uploads).await;
                remove_dir_contents(&staging).await?;
                remove_dir_contents(&uploads).await?;
                return Ok(json!({"files": files_a + files_b, "bytes": bytes_a + bytes_b}));
            }
            let report = state
                .services
                .strata()
                .purge_unreferenced_blobs(false)
                .await
                .map_err(ApiError::from)?;
            Ok(serde_json::to_value(report).unwrap_or_else(|_| json!({})))
        }
        "delivery" => {
            let mut removed = 0u64;
            for kind in [
                "delivery_rule",
                "delivery_endpoint",
                "delivery_record",
                "notification_entry",
                "notification_center_settings",
                "push_channel",
                "push_record",
                "channel",
                "publication",
                "delivery",
                "export",
                "upload",
                // 旧加工与发布数据：功能已下线，仍随该域清理。
                "publication",
                "input_set",
                "workflow",
                "workflow_version",
                "production_run",
                "production_attempt",
                "candidate",
                "review",
                "draft",
            ] {
                removed += state.store.delete_kind(kind).await?;
            }
            remove_dir_contents(&state.data_dir.join("uploads")).await?;
            Ok(json!({"removed": removed}))
        }
        "logs" | "audit" => {
            let removed = state.store.clear_records(domain).await?;
            Ok(json!({"removed": removed}))
        }
        "operations" => {
            let removed = state.store.clear_finished_operations().await?;
            Ok(json!({"removed": removed}))
        }
        "userSettings" => {
            // 用户资源与设置代际全部清空，随后重建默认设置与内置提示词、Skill。
            let mut removed = 0;
            for kind in ["model", "prompt", "skill", "channel", "settings_history"] {
                removed += state.store.delete_kind(kind).await?;
            }
            state.store.delete_kind("settings").await?;
            crate::settings::seed(state).await?;
            Ok(json!({"removed": removed}))
        }
        other => Err(ApiError::validation(
            "domains",
            "enum",
            format!("未知清空域: {other}"),
        )),
    }
}

async fn factory_root(data: &Path) -> PathBuf {
    data.join("factory")
}

pub(crate) async fn ensure_factory_snapshot(state: &AppState) -> Result<(), ApiError> {
    let root = factory_root(&state.data_dir).await.join("config");
    if root.exists() {
        return Ok(());
    }
    copy_dir(&state.config_dir, &root).await?;
    Ok(())
}

async fn restore_factory_config(state: &AppState) -> Result<(), ApiError> {
    let root = factory_root(&state.data_dir).await.join("config");
    if !root.exists() {
        return Err(ApiError::dependency("恢复出厂快照不存在"));
    }
    remove_dir_contents(&state.config_dir).await?;
    copy_dir(&root, &state.config_dir).await?;
    Ok(())
}

async fn copy_dir(from: &Path, to: &Path) -> Result<(), ApiError> {
    tokio::fs::create_dir_all(to)
        .await
        .map_err(|error| ApiError::internal(format!("创建目录失败: {error}")))?;
    let mut entries = tokio::fs::read_dir(from)
        .await
        .map_err(|error| ApiError::internal(format!("读取目录失败: {error}")))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| ApiError::internal(format!("读取目录项失败: {error}")))?
    {
        let target = to.join(entry.file_name());
        let meta = entry
            .metadata()
            .await
            .map_err(|error| ApiError::internal(format!("读取元数据失败: {error}")))?;
        if meta.is_dir() {
            Box::pin(copy_dir(&entry.path(), &target)).await?;
        } else {
            tokio::fs::copy(entry.path(), &target)
                .await
                .map_err(|error| ApiError::internal(format!("复制文件失败: {error}")))?;
        }
    }
    Ok(())
}

async fn remove_dir_contents(path: &Path) -> Result<(), ApiError> {
    let Ok(mut entries) = tokio::fs::read_dir(path).await else {
        return Ok(());
    };
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| ApiError::internal(format!("读取目录失败: {error}")))?
    {
        let target = entry.path();
        if entry
            .metadata()
            .await
            .map(|meta| meta.is_dir())
            .unwrap_or(false)
        {
            tokio::fs::remove_dir_all(&target)
                .await
                .map_err(|error| ApiError::internal(format!("删除目录失败: {error}")))?;
        } else {
            tokio::fs::remove_file(&target)
                .await
                .map_err(|error| ApiError::internal(format!("删除文件失败: {error}")))?;
        }
    }
    Ok(())
}
