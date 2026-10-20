use assembly::capacities;
use axum::Json;
use axum::extract::{Path as AxPath, Query, State};
use egress::circuit_open;
use serde::Deserialize;
use serde_json::{Value, json};
use strata_model::{AsOf, Expr, FacetKind, FacetRef, Query as StrataQuery, QueryFrom, VersionSel};

use crate::config_path::{list_whitelist_files, normalize_rel_path, resolve_config_file};
use crate::copy::{
    change_summary, circuit_label, health_chip_class, health_label, schedule_note, skip_reason_label,
};
use crate::error::{ApiError, ApiResult};
use crate::ConsoleState;

#[derive(Deserialize)]
pub struct AsOfQuery {
    #[serde(default)]
    as_of_snapshot: Option<u64>,
}

pub async fn meta(
    State(state): State<ConsoleState>,
    Query(q): Query<AsOfQuery>,
) -> ApiResult<Json<Value>> {
    let generation = state
        .services
        .call("config.generation", json!({}))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let read_only = q.as_of_snapshot.is_some();
    Ok(Json(json!({
        "httpListen": state.http_addr,
        "configGeneration": generation.get("generation"),
        "readOnly": read_only,
        "asOfSnapshot": q.as_of_snapshot,
        "consolePath": "/console/",
    })))
}

pub async fn timeline(State(state): State<ConsoleState>) -> ApiResult<Json<Value>> {
    let changes = state
        .services
        .call("strata_read.changes", json!({ "after": 0, "limit": 100 }))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let items: Vec<Value> = changes
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|row| {
            json!({
                "snapshot": row.get("snapshot"),
                "publishedAt": row.get("publishedAt"),
                "label": change_summary(&row),
            })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

pub async fn overview(
    State(state): State<ConsoleState>,
    Query(q): Query<AsOfQuery>,
) -> ApiResult<Json<Value>> {
    let historical = q.as_of_snapshot.is_some();
    let sources = source_summaries(&state).await?;
    let healthy = sources
        .iter()
        .filter(|s| s.get("healthStatus").and_then(Value::as_str) == Some("ok"))
        .count();
    let changes = state
        .services
        .call("strata_read.changes", json!({ "after": 0, "limit": 20 }))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let recent: Vec<Value> = changes
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|row| {
            json!({
                "snapshot": row.get("snapshot"),
                "publishedAt": row.get("publishedAt"),
                "summary": change_summary(&row),
            })
        })
        .collect();
    let c = capacities();
    Ok(Json(json!({
        "sourceCount": sources.len(),
        "healthyCount": healthy,
        "queueCapacities": {
            "free": c.free,
            "paid": c.paid,
            "render": c.render,
            "push": c.push,
        },
        "recentUpdates": recent,
        "sources": sources,
        "historicalMode": historical,
        "viewNote": historical.then_some("该视图显示实时状态"),
        "readOnly": historical,
    })))
}

pub async fn sources_list(
    State(state): State<ConsoleState>,
    Query(q): Query<AsOfQuery>,
) -> ApiResult<Json<Value>> {
    let historical = q.as_of_snapshot.is_some();
    let items = source_summaries(&state).await?;
    Ok(Json(json!({
        "items": items,
        "historicalMode": historical,
        "viewNote": historical.then_some("该视图显示实时状态"),
        "readOnly": historical,
    })))
}

pub async fn source_get(
    State(state): State<ConsoleState>,
    AxPath(id): AxPath<String>,
    Query(_q): Query<AsOfQuery>,
) -> ApiResult<Json<Value>> {
    let key = normalize_source_id(&id)?;
    let items = source_summaries(&state).await?;
    let Some(item) = items
        .into_iter()
        .find(|s| s.get("id") == Some(&Value::String(key.clone())))
    else {
        return Err(ApiError::not_found("信源不存在"));
    };
    Ok(Json(item))
}

#[derive(Deserialize)]
pub struct TriggerBody {
    #[serde(default)]
    confirm: bool,
}

pub async fn source_trigger(
    State(state): State<ConsoleState>,
    AxPath(id): AxPath<String>,
    body: Option<Json<TriggerBody>>,
) -> ApiResult<Json<Value>> {
    let key = normalize_source_id(&id)?;
    if body.is_some_and(|b| !b.confirm) {
        return Err(ApiError::bad_request("需要 confirm: true"));
    }
    let tasks = state
        .services
        .call(
            "planner.trigger",
            json!({ "source": key.strip_prefix("source/").unwrap_or(&key) }),
        )
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({ "tasks": tasks })))
}

#[derive(Deserialize)]
pub struct PatchSource {
    enabled: Option<bool>,
    #[serde(default)]
    confirm: bool,
}

pub async fn source_patch(
    State(state): State<ConsoleState>,
    AxPath(id): AxPath<String>,
    Json(body): Json<PatchSource>,
) -> ApiResult<Json<Value>> {
    if !body.confirm {
        return Err(ApiError::bad_request("需要 confirm: true"));
    }
    let key = normalize_source_id(&id)?;
    let enabled = body
        .enabled
        .ok_or_else(|| ApiError::bad_request("需要 enabled 字段"))?;
    let rel = format!("sources/{}.yaml", key.strip_prefix("source/").unwrap_or(&key));
    let rel_path = normalize_rel_path(&rel)?;
    let path = resolve_config_file(&state, &rel_path)?;
    let text = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| ApiError::not_found(e.to_string()))?;
    write_backup(&state, &rel, &text)?;
    let updated = set_enabled_line(&text, enabled);
    tokio::fs::write(&path, updated)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    validate_and_reload(&state).await?;
    Ok(Json(json!({ "ok": true, "enabled": enabled })))
}

#[derive(Deserialize)]
pub struct RecordsQuery {
    #[serde(default = "default_limit")]
    limit: u64,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    as_of_snapshot: Option<u64>,
}

fn default_limit() -> u64 {
    40
}

pub async fn records_list(
    State(state): State<ConsoleState>,
    Query(q): Query<RecordsQuery>,
) -> ApiResult<Json<Value>> {
    let limit = q.limit.clamp(1, 100) as u32;
    let mut where_expr = None;
    if let Some(source) = &q.source {
        let sid = if source.starts_with("source/") {
            source.clone()
        } else {
            format!("source/{source}")
        };
        where_expr = Some(Expr::try_from(json!({ "eq": { "field": "sourceId", "value": sid } }))
            .map_err(|_| ApiError::bad_request("信源筛选无效"))?);
    }
    let as_of = q.as_of_snapshot.map(AsOf::Snapshot);
    let query = StrataQuery {
        from: QueryFrom::Resources,
        r#where: where_expr,
        has_facets: vec![],
        expand: vec![],
        order_by: vec![],
        limit: Some(limit.saturating_mul(2).min(200)),
        cursor: q.cursor.clone(),
        search: None,
        select: vec![],
        as_of: as_of.clone(),
    };
    let page = state
        .services
        .strata()
        .query(query)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let needle = q.q.as_deref().map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());
    let mut items: Vec<Value> = page
        .rows
        .into_iter()
        .take(limit as usize)
        .map(|row| record_summary(&row))
        .collect();
    enrich_record_summaries(&state, &mut items, as_of.clone()).await?;
    if let Some(needle) = &needle {
        items.retain(|row| record_matches_query(row, needle));
    }
    Ok(Json(json!({
        "items": items,
        "nextCursor": page.next_cursor,
        "readOnly": q.as_of_snapshot.is_some(),
        "sourceOptions": source_filter_options(&state),
    })))
}

pub async fn record_get(
    State(state): State<ConsoleState>,
    AxPath(id): AxPath<String>,
    Query(q): Query<AsOfQuery>,
) -> ApiResult<Json<Value>> {
    let rid_text = if id.starts_with("r_") {
        id.clone()
    } else {
        format!("r_{id}")
    };
    let as_of = q.as_of_snapshot.map(AsOf::Snapshot);
    let detail = load_record_detail(&state, &rid_text, as_of).await?;
    Ok(Json(detail))
}

pub async fn governance_egress(
    State(state): State<ConsoleState>,
    Query(q): Query<AsOfQuery>,
) -> ApiResult<Json<Value>> {
    let historical = q.as_of_snapshot.is_some();
    let db = state
        .services
        .egress_gateway()
        .egress_db()
        .await
        .map_err(|e| ApiError::internal(e.message))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let domains: Vec<Value> = db
        .circuits
        .iter()
        .map(|(host, row)| {
            let open = circuit_open(&db, host, now);
            let state_label = if open {
                "open"
            } else if row.open_until.is_some() {
                "half-open"
            } else {
                "closed"
            };
            json!({
                "host": host,
                "circuit": state_label,
                "circuitLabel": circuit_label(state_label),
                "consecutiveFailures": row.consecutive,
                "cooldownMs": row.cooldown_ms,
            })
        })
        .collect();
    Ok(Json(json!({
        "domains": domains,
        "proxy": state.services.proxy_url(),
        "historicalMode": historical,
        "viewNote": historical.then_some("该视图显示实时状态"),
        "readOnly": historical,
    })))
}

#[derive(Deserialize)]
pub struct ConfirmBody {
    #[serde(default)]
    confirm: bool,
}

pub async fn reset_circuit(
    State(state): State<ConsoleState>,
    AxPath(host): AxPath<String>,
    body: Option<Json<ConfirmBody>>,
) -> ApiResult<Json<Value>> {
    if body.is_some_and(|b| !b.confirm) {
        return Err(ApiError::bad_request("需要 confirm: true"));
    }
    state
        .services
        .egress_gateway()
        .reset_circuit(&host)
        .await
        .map_err(|e| ApiError::internal(e.message))?;
    Ok(Json(json!({ "ok": true, "host": host })))
}

pub async fn config_summary(State(state): State<ConsoleState>) -> ApiResult<Json<Value>> {
    let generation = state
        .services
        .call("config.generation", json!({}))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let diff = state
        .services
        .call("config_inspect.diff", json!({}))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let diff_len = diff.as_array().map(|a| a.len()).unwrap_or(0);
    Ok(Json(json!({
        "generation": generation.get("generation"),
        "pendingDiffCount": diff_len,
    })))
}

pub async fn config_validate(State(state): State<ConsoleState>) -> ApiResult<Json<Value>> {
    state
        .services
        .call("config_inspect.validate", json!({}))
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn config_reload(State(state): State<ConsoleState>) -> ApiResult<Json<Value>> {
    let value = state
        .services
        .call("config.load", json!({}))
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(Json(value))
}

pub async fn config_files_list(State(state): State<ConsoleState>) -> ApiResult<Json<Value>> {
    let paths = list_whitelist_files(&state)?;
    let snapshot = state.services.config().applied();
    let groups = config_file_groups(&paths, snapshot.as_ref());
    Ok(Json(json!({ "items": paths, "groups": groups })))
}

pub async fn config_file_get(
    State(state): State<ConsoleState>,
    AxPath(path): AxPath<String>,
) -> ApiResult<Json<Value>> {
    let rel = normalize_rel_path(&path)?;
    let file = resolve_config_file(&state, &rel)?;
    let text = tokio::fs::read_to_string(&file)
        .await
        .map_err(|e| ApiError::not_found(e.to_string()))?;
    let has_backup = backup_path(&state, &path).is_file();
    Ok(Json(json!({
        "path": path,
        "text": text,
        "canRollback": has_backup,
    })))
}

#[derive(Deserialize)]
pub struct FileValidateBody {
    path: String,
    text: String,
}

#[derive(Deserialize)]
pub struct FilePathBody {
    path: String,
    #[serde(default)]
    confirm: bool,
}

pub async fn config_file_validate_text(
    State(state): State<ConsoleState>,
    Json(body): Json<FileValidateBody>,
) -> ApiResult<Json<Value>> {
    let rel = normalize_rel_path(&body.path)?;
    let file = resolve_config_file(&state, &rel)?;
    let original = if file.is_file() {
        tokio::fs::read_to_string(&file)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?
    } else {
        String::new()
    };
    tokio::fs::write(&file, &body.text)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let result = state
        .services
        .call("config_inspect.validate", json!({}))
        .await;
    tokio::fs::write(&file, original)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    match result {
        Ok(_) => Ok(Json(json!({ "ok": true }))),
        Err(e) => {
            let msg = e.to_string();
            let line = validation_line_hint(&msg);
            Err(ApiError::bad_request(if let Some(line) = line {
                format!("第 {line} 行：{msg}")
            } else {
                msg
            }))
        }
    }
}

pub async fn config_file_put(
    State(state): State<ConsoleState>,
    AxPath(path): AxPath<String>,
    Json(body): Json<PutFileBody>,
) -> ApiResult<Json<Value>> {
    if !body.confirm {
        return Err(ApiError::bad_request("需要 confirm: true"));
    }
    let rel = normalize_rel_path(&path)?;
    let file = resolve_config_file(&state, &rel)?;
    let original = if file.is_file() {
        tokio::fs::read_to_string(&file)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?
    } else {
        String::new()
    };
    write_backup(&state, &path.to_string(), &original)?;
    tokio::fs::write(&file, &body.text)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if let Err(e) = validate_and_reload(&state).await {
        tokio::fs::write(&file, original)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        return Err(e);
    }
    Ok(Json(json!({ "ok": true, "path": path })))
}

#[derive(Deserialize)]
pub struct PutFileBody {
    text: String,
    #[serde(default)]
    confirm: bool,
}

pub async fn config_file_rollback(
    State(state): State<ConsoleState>,
    Json(body): Json<FilePathBody>,
) -> ApiResult<Json<Value>> {
    if !body.confirm {
        return Err(ApiError::bad_request("需要 confirm: true"));
    }
    let rel = normalize_rel_path(&body.path)?;
    let file = resolve_config_file(&state, &rel)?;
    let backup = backup_path(&state, &body.path);
    if !backup.is_file() {
        return Err(ApiError::not_found("没有可回滚的备份"));
    }
    let text = tokio::fs::read_to_string(&backup)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    tokio::fs::write(&file, text)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    validate_and_reload(&state).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn validate_and_reload(state: &ConsoleState) -> ApiResult<()> {
    state
        .services
        .call("config_inspect.validate", json!({}))
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    state
        .services
        .call("config.load", json!({}))
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(())
}

fn backup_path(state: &ConsoleState, rel_path: &str) -> std::path::PathBuf {
    let safe = rel_path.replace('/', "__");
    state
        .services
        .data_dir()
        .join(".console-config-backups")
        .join(safe)
}

fn write_backup(state: &ConsoleState, rel_path: &str, text: &str) -> ApiResult<()> {
    let path = backup_path(state, rel_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ApiError::internal(e.to_string()))?;
    }
    std::fs::write(&path, text).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(())
}

fn normalize_source_id(id: &str) -> ApiResult<String> {
    let key = if id.starts_with("source/") {
        id.to_string()
    } else {
        format!("source/{id}")
    };
    if key.contains("..") {
        return Err(ApiError::bad_request("信源 ID 无效"));
    }
    Ok(key)
}

async fn source_summaries(state: &ConsoleState) -> ApiResult<Vec<Value>> {
    let snapshot = state.services.config().applied();
    let mut out = Vec::new();
    for (id, module) in &snapshot.modules {
        if module.get("kind").and_then(Value::as_str) != Some("source") {
            continue;
        }
        let enabled = module.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        let name = module
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(id)
            .to_string();
        let accessor = module
            .pointer("/access/accessor")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let source_key = id.strip_prefix("source/").unwrap_or(id);
        let status = state
            .services
            .call("planner.next", json!({ "source": source_key }))
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        let health = state
            .services
            .call("health.status", json!({ "source": source_key }))
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        let health_status = health
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("ok");
        let alert = health.get("alert").and_then(Value::as_bool).unwrap_or(false);
        let skip = status.get("skipReason").and_then(Value::as_str);
        let budget_used = status.get("budgetUsed").and_then(Value::as_u64);
        let note = schedule_note(
            enabled,
            skip,
            budget_used,
            status.get("nextAt"),
        );
        let chip = health_chip_class(health_status, enabled, skip);
        out.push(json!({
            "id": id,
            "name": name,
            "enabled": enabled,
            "accessor": accessor,
            "healthStatus": health_status,
            "healthLabel": health_label(health_status, alert),
            "statusChipClass": chip,
            "skipReasonLabel": skip_reason_label(skip),
            "scheduleNote": note,
            "nextAt": status.get("nextAt"),
            "lastYieldAt": status.get("lastYieldAt"),
            "todayYieldCount": status.get("todayYieldCount"),
            "lastError": status.get("lastError"),
        }));
    }
    out.sort_by(|a, b| {
        a.get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .cmp(b.get("id").and_then(Value::as_str).unwrap_or(""))
    });
    Ok(out)
}

fn record_summary(row: &Value) -> Value {
    json!({
        "id": row.get("rid"),
        "sourceId": row.get("sourceId"),
        "updatedAt": row.get("updatedAt"),
        "tombstone": row.get("tombstone"),
        "title": Value::Null,
        "sourceName": Value::Null,
        "publishedAt": Value::Null,
        "collectedAt": row.get("updatedAt"),
    })
}

fn record_matches_query(row: &Value, needle: &str) -> bool {
    let rid = row.get("id").and_then(Value::as_str).unwrap_or("");
    let sid = row.get("sourceId").and_then(Value::as_str).unwrap_or("");
    let title = row.get("title").and_then(Value::as_str).unwrap_or("");
    let source_name = row.get("sourceName").and_then(Value::as_str).unwrap_or("");
    rid.to_lowercase().contains(needle)
        || sid.to_lowercase().contains(needle)
        || title.to_lowercase().contains(needle)
        || source_name.to_lowercase().contains(needle)
}

fn source_name_map(state: &ConsoleState) -> std::collections::HashMap<String, String> {
    let snapshot = state.services.config().applied();
    let mut out = std::collections::HashMap::new();
    for (id, module) in &snapshot.modules {
        if module.get("kind").and_then(Value::as_str) != Some("source") {
            continue;
        }
        let name = module
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(id)
            .to_string();
        out.insert(id.clone(), name);
    }
    out
}

fn source_filter_options(state: &ConsoleState) -> Vec<Value> {
    source_name_map(state)
        .into_iter()
        .map(|(id, name)| json!({ "id": id, "name": name }))
        .collect()
}

async fn enrich_record_summaries(
    state: &ConsoleState,
    items: &mut [Value],
    as_of: Option<AsOf>,
) -> ApiResult<()> {
    let names = source_name_map(state);
    let strata = state.services.strata();
    for item in items {
        let rid = item
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if rid.is_empty() {
            continue;
        }
        let parsed = strata_model::Rid::parse(&rid).map_err(|_| ApiError::bad_request("记录无效"))?;
        let discovery = facet_get_at(
            strata.as_ref(),
            &parsed,
            FacetKind::Struct,
            Some("discovery"),
            as_of.clone(),
        )
        .await?;
        let fields = parse_discovery(&discovery);
        if let Some(title) = fields.title {
            item["title"] = json!(title);
        }
        if let Some(published) = fields.published_at {
            item["publishedAt"] = json!(published);
        }
        let sid = discovery
            .as_ref()
            .and_then(|f| f.value.as_ref())
            .and_then(|v| v.pointer("/sourceId"))
            .and_then(Value::as_str)
            .or_else(|| item.get("sourceId").and_then(Value::as_str))
            .map(str::to_string);
        if let Some(sid) = sid {
            let label = names.get(&sid).cloned().unwrap_or_else(|| sid.clone());
            item["sourceId"] = json!(sid);
            item["sourceName"] = json!(label);
        }
    }
    Ok(())
}

fn config_file_groups(paths: &[String], snapshot: &config::Snapshot) -> Vec<Value> {
    let mut sources = Vec::new();
    let mut presets = Vec::new();
    for path in paths {
        let label = file_display_name(path, snapshot);
        let entry = json!({ "path": path, "label": label });
        if path.starts_with("sources/") {
            sources.push(entry);
        } else if path.starts_with("presets/") {
            presets.push(entry);
        }
    }
    sources.sort_by(|a, b| {
        a["label"]
            .as_str()
            .unwrap_or("")
            .cmp(b["label"].as_str().unwrap_or(""))
    });
    presets.sort_by(|a, b| {
        a["label"]
            .as_str()
            .unwrap_or("")
            .cmp(b["label"].as_str().unwrap_or(""))
    });
    vec![
        json!({ "id": "sources", "label": "信源", "items": sources }),
        json!({ "id": "presets", "label": "策略预设", "items": presets }),
    ]
}

fn file_display_name(path: &str, snapshot: &config::Snapshot) -> String {
    if let Some(stem) = path
        .strip_prefix("sources/")
        .and_then(|p| p.strip_suffix(".yaml").or_else(|| p.strip_suffix(".yml")))
    {
        let id = format!("source/{stem}");
        if let Some(module) = snapshot.modules.get(&id) {
            return module
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(stem)
                .to_string();
        }
        return stem.to_string();
    }
    if let Some(name) = path.strip_prefix("presets/") {
        return name
            .strip_suffix(".yaml")
            .or_else(|| name.strip_suffix(".yml"))
            .unwrap_or(name)
            .to_string();
    }
    path.to_string()
}

fn validation_line_hint(message: &str) -> Option<u32> {
    for token in message.split_whitespace() {
        if let Some(rest) = token.strip_prefix("line:") {
            if let Ok(line) = rest.parse::<u32>() {
                return Some(line);
            }
        }
        if let Some(rest) = token.strip_prefix("line") {
            if let Ok(line) = rest.trim_matches(':').parse::<u32>() {
                return Some(line);
            }
        }
    }
    None
}

async fn load_record_detail(
    state: &ConsoleState,
    rid_text: &str,
    as_of: Option<AsOf>,
) -> ApiResult<Value> {
    let rid = strata_model::Rid::parse(rid_text)
        .map_err(|_| ApiError::bad_request("记录 ID 无效"))?;
    let strata = state.services.strata();
    let discovery = facet_get_at(
        strata.as_ref(),
        &rid,
        FacetKind::Struct,
        Some("discovery"),
        as_of.clone(),
    )
    .await?;
    let page = facet_get_at(
        strata.as_ref(),
        &rid,
        FacetKind::Struct,
        Some("page"),
        as_of.clone(),
    )
    .await?;
    let fields = parse_discovery(&discovery);
    let body_preview = page
        .as_ref()
        .and_then(|f| f.value.as_ref())
        .and_then(|v| v.get("readable"))
        .and_then(Value::as_str)
        .or_else(|| {
            page.as_ref()
                .and_then(|f| f.value.as_ref())
                .and_then(|v| v.get("text"))
                .and_then(Value::as_str)
        })
        .map(|s| truncate(s, 2000));
    let kinds = state
        .services
        .call(
            "strata_read.kinds",
            json!({ "facet": format!("{rid_text}@struct:discovery#latest") }),
        )
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let names = source_name_map(state);
    let source_id = discovery
        .as_ref()
        .and_then(|f| f.value.as_ref())
        .and_then(|v| v.pointer("/sourceId"))
        .and_then(Value::as_str);
    let source_name = source_id.and_then(|sid| names.get(sid).cloned());
    Ok(json!({
        "id": rid_text,
        "title": fields.title,
        "url": fields.url,
        "publishedAt": fields.published_at,
        "summary": fields.summary,
        "author": fields.author,
        "sourceId": source_id,
        "sourceName": source_name,
        "collectedAt": discovery.as_ref().and_then(|f| f.value.as_ref()).and_then(|v| v.get("updatedAt")).or_else(|| page.as_ref().and_then(|f| f.value.as_ref()).and_then(|v| v.get("updatedAt"))),
        "bodyPreview": body_preview,
        "readOnly": as_of.is_some(),
        "structured": discovery.as_ref().and_then(|f| f.value.clone()),
        "technical": {
            "rid": rid_text,
            "facetKinds": kinds,
            "pageFacet": page.as_ref().and_then(|f| f.value.clone()),
        },
    }))
}

async fn facet_get(
    strata: &strata_client::Strata,
    reference: &FacetRef,
) -> ApiResult<Option<strata_client::Facet>> {
    match strata.get(reference, true).await {
        Ok(facet) => Ok(facet),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("Not found") || msg.contains("not found") {
                Ok(None)
            } else {
                Err(ApiError::internal(msg))
            }
        }
    }
}

async fn facet_get_at(
    strata: &strata_client::Strata,
    rid: &strata_model::Rid,
    kind: FacetKind,
    variant: Option<&str>,
    as_of: Option<AsOf>,
) -> ApiResult<Option<strata_client::Facet>> {
    let variant_owned = variant.map(str::to_string);
    let version = match as_of {
        None => VersionSel::Latest,
        Some(AsOf::Snapshot(seq)) => {
            let facet_latest = FacetRef::new(
                rid.clone(),
                kind.clone(),
                variant_owned.clone(),
                VersionSel::Latest,
            )
            .map_err(|_| ApiError::bad_request("记录 ID 无效"))?;
            let history = strata.history(&facet_latest).await.map_err(|e| ApiError::internal(e.to_string()))?;
            let picked = history
                .iter()
                .filter(|item| item.seq <= seq)
                .max_by_key(|item| item.seq);
            picked
                .map(|item| VersionSel::Exact(item.version))
                .unwrap_or(VersionSel::Latest)
        }
        Some(_) => VersionSel::Latest,
    };
    let reference = FacetRef::new(rid.clone(), kind.clone(), variant_owned, version)
        .map_err(|_| ApiError::bad_request("记录 ID 无效"))?;
    facet_get(strata, &reference).await
}

struct DiscoveryFields {
    title: Option<String>,
    url: Option<String>,
    published_at: Option<String>,
    summary: Option<String>,
    author: Option<String>,
}

fn parse_discovery(facet: &Option<strata_client::Facet>) -> DiscoveryFields {
    let value = facet.as_ref().and_then(|f| f.value.as_ref());
    let hints = value.and_then(|v| v.pointer("/hints"));
    let title = hints.and_then(|h| h.get("title")).and_then(Value::as_str).map(String::from);
    let published_at = hints
        .and_then(|h| h.get("publishedAt"))
        .map(|v| v.as_str().map(String::from).unwrap_or_else(|| v.to_string()));
    let summary = hints.and_then(|h| h.get("summary")).and_then(Value::as_str).map(String::from);
    let author = hints.and_then(|h| h.get("author")).and_then(Value::as_str).map(String::from);
    let url = value
        .and_then(|v| v.pointer("/locator/url"))
        .and_then(Value::as_str)
        .map(String::from)
        .or_else(|| value.and_then(|v| v.pointer("/locator/href")).and_then(Value::as_str).map(String::from));
    DiscoveryFields {
        title,
        url,
        published_at,
        summary,
        author,
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

fn set_enabled_line(yaml: &str, enabled: bool) -> String {
    let target = format!("enabled: {}", enabled);
    let mut found = false;
    let mut lines: Vec<String> = yaml
        .lines()
        .map(|line| {
            if line.trim_start().starts_with("enabled:") {
                found = true;
                target.clone()
            } else {
                line.to_string()
            }
        })
        .collect();
    if !found {
        if let Some(pos) = lines.iter().position(|l| l.starts_with("id:")) {
            lines.insert(pos + 1, target);
        } else {
            lines.insert(0, target);
        }
    }
    let mut out = lines.join("\n");
    if yaml.ends_with('\n') {
        out.push('\n');
    }
    out
}
