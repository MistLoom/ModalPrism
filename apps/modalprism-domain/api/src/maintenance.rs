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

/// 配置域现状：可预演、可核算的那部分。
///
/// 配置文件不是清空域（它只能被出厂基线替换），但用户有权在执行前知道「会重建多少
/// 声明、会退役哪些遗留目录、出厂基线在不在」。
async fn config_state(state: &AppState) -> Value {
    let root = &state.config_dir;
    let mut declarations = 0u64;
    for name in config::declaration_paths() {
        let path = root.join(name);
        if path.is_file() {
            declarations += 1;
        } else if path.is_dir() {
            declarations += count_yaml_files(&path).await;
        }
    }
    let retired: Vec<Value> = config::retired_paths()
        .into_iter()
        .filter(|name| root.join(name).exists())
        .map(|name| json!({"path": name, "since": config::retired_since(name)}))
        .collect();
    let baseline = factory_root(&state.data_dir).await.join("config");
    let baseline_available =
        baseline.is_dir() && config::validate(&baseline, &crate::catalog::options()).is_ok();
    json!({
        "declarations": declarations,
        "retiredPaths": retired,
        "baselineAvailable": baseline_available,
    })
}

/// 递归数 YAML 声明文件；读不到的目录直接跳过，预演不因为权限问题失败。
async fn count_yaml_files(path: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let Ok(meta) = entry.metadata().await else {
                continue;
            };
            if meta.is_dir() {
                stack.push(path);
                continue;
            }
            if path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext == "yaml" || ext == "yml")
            {
                total += 1;
            }
        }
    }
    total
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
        // 配置不是清空域：这里给出「重建哪些、退役哪些、基线在不在」供执行前核对。
        "config": config_state(state).await,
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
    // 排空结束才真正开始清数据：先推一次进度，让控制台知道已经跨过等待阶段。
    crate::operations::progress(state, operation_id, 0.2).await;
    let total = domains.len().max(1);
    for (index, domain) in domains.iter().enumerate() {
        if crate::operations::check_cancel(state, operation_id)
            .await
            .is_err()
        {
            cancelled = true;
            break;
        }
        // 按域推进：任务卡片的进度条能看得出正在清哪一段。
        crate::operations::progress(state, operation_id, 0.2 + 0.5 * index as f64 / total as f64)
            .await;
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
    crate::operations::progress(state, operation_id, 0.72).await;
    if factory {
        // 记下恢复前还欠清理的已下线目录：它们会被基线替换与退役清理一并抹掉。
        let retired_before: Vec<&str> = config::retired_paths()
            .into_iter()
            .filter(|name| state.config_dir.join(name).exists())
            .collect();
        report["retiredConfigPaths"] = json!(retired_before);
        state
            .store
            .reset_documents_keep_operation(operation_id)
            .await?;
        // 先恢复配置目录：内置资源（提示词、Skill、模型）要从恢复后的声明里重建。
        if let Err(error) = restore_factory_config(state, operation_id).await {
            failed.push(json!({"domain": "config", "code": error.code, "message": error.message}));
        }
        // 内置资源与设置是运行底线：不管配置恢复成败都要重建，不能让系统落到
        // 「设置尚未初始化」或没有默认项目的裸状态。
        if let Err(error) = crate::settings::seed(state).await {
            failed.push(json!({"domain": "userSettings", "code": error.code, "message": error.message}));
        }
        if let Err(error) = crate::projects::seed(state).await {
            failed.push(json!({"domain": "projects", "code": error.code, "message": error.message}));
        }
        if failed.is_empty() {
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
        crate::operations::progress(state, operation_id, 0.9).await;
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
    // 恢复后的配置域现状：声明数量、遗留目录（应为空）、基线是否可用。
    report["config"] = config_state(state).await;
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
    // 只有通过校验的配置才配当出厂基线：把一份坏配置冻成快照，恢复出厂就永远失败。
    if let Err(error) = config::validate(&state.config_dir, &crate::catalog::options()) {
        tracing::warn!(detail = %error, "当前配置未通过校验，暂不建立出厂快照");
        return Ok(());
    }
    copy_dir(&state.config_dir, &root).await?;
    // 基线里不留已下线目录的副本，否则它会被恢复出厂一次次复制回来。
    strip_retired(&root).await;
    Ok(())
}

/// 从配置副本里剔除已下线目录。
///
/// 出厂基线可能早于退役登记：`workflows/` 这类历史目录会原样藏在快照里，清理必须
/// 覆盖「即将落位的副本」，不能只依赖启动时的清理。
async fn strip_retired(root: &Path) {
    for rel in config::retired_paths() {
        let path = root.join(rel);
        if !path.exists() {
            continue;
        }
        let outcome = if path.is_dir() {
            tokio::fs::remove_dir_all(&path).await
        } else {
            tokio::fs::remove_file(&path).await
        };
        match outcome {
            Ok(()) => tracing::warn!(path = %path.display(), "已下线配置目录已从副本剔除"),
            Err(error) => {
                tracing::error!(path = %path.display(), detail = %error, "剔除已下线目录失败")
            }
        }
    }
}

/// 用出厂快照替换配置目录并重新加载。
///
/// 顺序保证「要么换成能加载的出厂配置，要么保留原配置」：先校验快照，再复制到临时目录校验
/// 副本，最后才清空 live 目录落位；落位或热加载失败时用替换前的备份回滚并再次加载。
/// 恢复出厂本身失败不该把系统变成起不来的状态。
async fn restore_factory_config(state: &AppState, operation_id: &str) -> Result<(), ApiError> {
    let root = factory_root(&state.data_dir).await.join("config");
    if !root.is_dir() {
        return Err(ApiError::dependency("恢复出厂快照不存在"));
    }
    let options = crate::catalog::options();
    if let Err(error) = config::validate(&root, &options) {
        tracing::error!(detail = %error, "出厂配置快照无法通过校验");
        return Err(ApiError::dependency(
            "出厂配置快照无法通过校验，已保留当前配置",
        ));
    }
    let backup = state
        .data_dir
        .join("backup")
        .join(format!("config-before-factory-{operation_id}"));
    // 重试会复用同一个操作标识：先清掉上一次留下的副本，避免陈旧文件混入比对。
    let _ = tokio::fs::remove_dir_all(&backup).await;
    copy_dir(&state.config_dir, &backup).await?;
    tracing::info!(path = %backup.display(), "配置已备份");
    let staging = state
        .data_dir
        .join("backup")
        .join(format!("config-factory-{operation_id}"));
    let _ = tokio::fs::remove_dir_all(&staging).await;
    if let Err(error) = copy_dir(&root, &staging).await {
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err(error);
    }
    strip_retired(&staging).await;
    if let Err(error) = config::validate(&staging, &options) {
        tracing::error!(detail = %error, "出厂配置副本无法通过校验");
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err(ApiError::dependency(
            "出厂配置快照无法通过校验，已保留当前配置",
        ));
    }
    let placed = async {
        remove_dir_contents(&state.config_dir).await?;
        copy_dir(&staging, &state.config_dir).await
    }
    .await;
    let _ = tokio::fs::remove_dir_all(&staging).await;
    if let Err(error) = placed {
        tracing::error!(detail = %error.message, "恢复出厂配置落位失败");
        rollback_config(state, &backup).await;
        return Err(ApiError::dependency("恢复出厂配置失败，已回滚到原配置"));
    }
    if let Err(error) = state.services.config_reload() {
        tracing::error!(detail = %error, "恢复出厂后热加载失败");
        rollback_config(state, &backup).await;
        return Err(ApiError::dependency(
            "恢复出厂配置后热加载失败，已回滚到原配置",
        ));
    }
    Ok(())
}

/// 用备份覆盖回配置目录并重新加载。回滚自身失败只记日志：此时已没有更安全的动作。
async fn rollback_config(state: &AppState, backup: &Path) {
    if let Err(error) = remove_dir_contents(&state.config_dir).await {
        tracing::error!(detail = %error, "回滚时清空配置目录失败");
        return;
    }
    if let Err(error) = copy_dir(backup, &state.config_dir).await {
        tracing::error!(detail = %error, "回滚时复制配置失败");
        return;
    }
    if let Err(error) = state.services.config_reload() {
        tracing::error!(detail = %error, "回滚后热加载失败");
    }
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
