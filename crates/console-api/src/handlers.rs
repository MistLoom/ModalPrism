use std::path::{Component, Path};

use assembly::capacities;
use axum::Json;
use axum::extract::{Path as AxPath, Query, State};
use egress::circuit_open;
use serde::Deserialize;
use serde_json::{Value, json};
use strata_model::{FacetRef, Query as StrataQuery, QueryFrom, VersionSel};

use crate::error::{ApiError, ApiResult};
use crate::ConsoleState;

pub async fn meta(State(state): State<ConsoleState>) -> ApiResult<Json<Value>> {
    let generation = state
        .services
        .call("config.generation", json!({}))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let snapshot = latest_snapshot_seq(&state).await?;
    Ok(Json(json!({
        "httpListen": state.http_addr,
        "configGeneration": generation.get("generation"),
        "latestSnapshotSeq": snapshot,
        "consolePath": "/console/",
    })))
}

pub async fn overview(State(state): State<ConsoleState>) -> ApiResult<Json<Value>> {
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
        "recentChanges": changes,
        "sources": sources,
    })))
}

pub async fn sources_list(State(state): State<ConsoleState>) -> ApiResult<Json<Value>> {
    let items = source_summaries(&state).await?;
    Ok(Json(json!({ "items": items })))
}

pub async fn source_get(
    State(state): State<ConsoleState>,
    AxPath(id): AxPath<String>,
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
        .call("planner.trigger", json!({ "source": key.strip_prefix("source/").unwrap_or(&key) }))
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
    let path = state.services.config_dir().join(&rel);
    if !path.is_file() {
        return Err(ApiError::not_found("配置文件不存在"));
    }
    let text = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let updated = set_enabled_line(&text, enabled);
    tokio::fs::write(&path, updated)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
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
    Ok(Json(json!({ "ok": true, "enabled": enabled })))
}

#[derive(Deserialize)]
pub struct RecordsQuery {
    #[serde(default = "default_limit")]
    limit: u64,
}

fn default_limit() -> u64 {
    40
}

pub async fn records_list(
    State(state): State<ConsoleState>,
    Query(q): Query<RecordsQuery>,
) -> ApiResult<Json<Value>> {
    let limit = q.limit.clamp(1, 100) as u32;
    let query = StrataQuery {
        from: QueryFrom::Resources,
        r#where: None,
        has_facets: vec![],
        expand: vec![],
        order_by: vec![],
        limit: Some(limit),
        cursor: None,
        search: None,
        select: vec![],
        as_of: None,
    };
    let page = state
        .services
        .strata()
        .query(query)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let items: Vec<Value> = page
        .rows
        .into_iter()
        .map(|row| {
            json!({
                "id": row.get("rid"),
                "sourceId": row.get("sourceId"),
                "updatedAt": row.get("updatedAt"),
                "tombstone": row.get("tombstone"),
            })
        })
        .collect();
    Ok(Json(json!({
        "items": items,
        "nextCursor": page.next_cursor,
    })))
}

pub async fn record_get(
    State(state): State<ConsoleState>,
    AxPath(id): AxPath<String>,
) -> ApiResult<Json<Value>> {
    let rid_text = if id.starts_with("r_") {
        id.clone()
    } else {
        format!("r_{id}")
    };
    let reference = FacetRef::new(
        strata_model::Rid::parse(&rid_text).map_err(|_| ApiError::bad_request("记录 ID 无效"))?,
        strata_model::FacetKind::Struct,
        Some("discovery".into()),
        VersionSel::Latest,
    )
    .map_err(|_| ApiError::bad_request("记录 ID 无效"))?;
    let discovery = state
        .services
        .strata()
        .get(&reference, true)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let kinds = state
        .services
        .call(
            "strata_read.kinds",
            json!({ "facet": format!("{rid_text}@struct:discovery#latest") }),
        )
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({
        "id": rid_text,
        "discovery": discovery.map(|f| f.value),
        "facetKinds": kinds,
    })))
}

pub async fn governance_egress(State(state): State<ConsoleState>) -> ApiResult<Json<Value>> {
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
                "consecutiveFailures": row.consecutive,
                "cooldownMs": row.cooldown_ms,
            })
        })
        .collect();
    Ok(Json(json!({
        "domains": domains,
        "proxy": state.services.proxy_url(),
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
    let value = state
        .services
        .call("config_inspect.validate", json!({}))
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(Json(value))
}

pub async fn config_reload(State(state): State<ConsoleState>) -> ApiResult<Json<Value>> {
    let value = state
        .services
        .call("config.load", json!({}))
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(Json(value))
}

pub async fn config_file_get(
    State(state): State<ConsoleState>,
    AxPath(path): AxPath<String>,
) -> ApiResult<Json<Value>> {
    let file = safe_config_path(&state, &path)?;
    let text = tokio::fs::read_to_string(&file)
        .await
        .map_err(|e| ApiError::not_found(e.to_string()))?;
    Ok(Json(json!({ "path": path, "text": text })))
}

pub async fn config_file_put(
    State(state): State<ConsoleState>,
    AxPath(path): AxPath<String>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let text = body
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::bad_request("需要 text 字段"))?;
    let file = safe_config_path(&state, &path)?;
    tokio::fs::write(&file, text)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}

fn safe_config_path(state: &ConsoleState, path: &str) -> ApiResult<std::path::PathBuf> {
    let rel = Path::new(path);
    if rel
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::RootDir | Component::Prefix(_)))
    {
        return Err(ApiError::bad_request("路径无效"));
    }
    let allowed = rel.starts_with("sources")
        || rel.starts_with("presets")
        || rel.starts_with("profiles")
        || rel.starts_with("accessors")
        || rel == Path::new("system.yaml");
    if !allowed {
        return Err(ApiError::bad_request("路径不在允许范围内"));
    }
    Ok(state.services.config_dir().join(rel))
}

fn normalize_source_id(id: &str) -> ApiResult<String> {
    let key = if id.starts_with("source/") {
        id.to_string()
    } else {
        format!("source/{id}")
    };
    if key.contains("..") || key.contains('/') && key.split('/').count() > 2 {
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
        let scheduled = module
            .pointer("/access/locator/type")
            .and_then(Value::as_str)
            != Some("channel");
        out.push(json!({
            "id": id,
            "name": name,
            "enabled": enabled,
            "accessor": accessor,
            "scheduled": scheduled,
            "healthStatus": health.get("status"),
            "alert": health.get("alert"),
            "nextAt": status.get("nextAt"),
            "intervalMs": status.get("intervalMs"),
            "skipReason": status.get("skipReason"),
            "budgetUsed": status.get("budgetUsed"),
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

async fn latest_snapshot_seq(state: &ConsoleState) -> ApiResult<Option<u64>> {
    let changes = state
        .services
        .call("strata_read.changes", json!({ "after": 0, "limit": 1 }))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if let Some(arr) = changes.as_array() {
        if let Some(first) = arr.first() {
            return Ok(first.get("seq").and_then(Value::as_u64));
        }
    }
    Ok(None)
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
