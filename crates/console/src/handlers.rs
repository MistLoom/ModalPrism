use collector_runtime::plans::capacities;
use crate::context;
use crate::invoke;
use crate::strata_port;
use serde::Deserialize;
use serde_json::{Value, json};
use strata_model::{AsOf, Expr, FacetKind, Query as StrataQuery, QueryFrom};

use crate::config_path::{list_whitelist_files, normalize_rel_path, resolve_config_file};
use crate::copy::{
    accessor_label, change_summary, circuit_label, health_chip_class, health_label, schedule_note,
    skip_reason_label,
};
use crate::error::{ConsoleError, ConsoleResult};
use crate::ConsoleContext;

#[derive(Deserialize)]
pub struct AsOfQuery {
    #[serde(default)]
    as_of_snapshot: Option<u64>,
}

pub async fn meta(
    ctx: &ConsoleContext,
    q: AsOfQuery,
) -> ConsoleResult<Value> {
    let generation = invoke::call(&ctx.gateway, "config.generation", json!({}))
        .await
        .map_err(|e| ConsoleError::internal(e.to_string()))?;
    let read_only = q.as_of_snapshot.is_some();
    Ok(json!({
        "httpListen": ctx.http_addr,
        "configGeneration": generation.get("generation"),
        "readOnly": read_only,
        "asOfSnapshot": q.as_of_snapshot,
        "consolePath": "/console/",
    }))
}

pub async fn timeline(ctx: &ConsoleContext) -> ConsoleResult<Value> {
    let changes = invoke::call(&ctx.gateway, "strata_read.changes", json!({ "after": 0, "limit": 100 }))
        .await
        .map_err(|e| ConsoleError::internal(e.to_string()))?;
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
    Ok(json!({ "items": items }))
}

pub async fn overview(
    ctx: &ConsoleContext,
    q: AsOfQuery,
) -> ConsoleResult<Value> {
    let historical = q.as_of_snapshot.is_some();
    let sources = source_summaries(ctx).await?;
    let healthy = sources
        .iter()
        .filter(|s| {
            s.get("enabled").and_then(Value::as_bool).unwrap_or(true)
                && s.get("healthStatus").and_then(Value::as_str) == Some("ok")
                && s.get("skipReasonLabel").is_none()
        })
        .count();
    let disabled_count = sources
        .iter()
        .filter(|s| !s.get("enabled").and_then(Value::as_bool).unwrap_or(true))
        .count();
    let attention_count = sources
        .iter()
        .filter(|s| {
            s.get("enabled").and_then(Value::as_bool).unwrap_or(true)
                && (s.get("skipReasonLabel").is_some()
                    || s.get("healthStatus").and_then(Value::as_str) != Some("ok"))
        })
        .count();
    let record_stats = record_stats_by_source(ctx).await?;
    let changes = invoke::call(&ctx.gateway, "strata_read.changes", json!({ "after": 0, "limit": 20 }))
        .await
        .map_err(|e| ConsoleError::internal(e.to_string()))?;
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
    let sources = merge_source_record_stats(sources, &record_stats);
    Ok(json!({
        "sourceCount": sources.len(),
        "healthyCount": healthy,
        "disabledCount": disabled_count,
        "attentionCount": attention_count,
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
    }))
}

pub async fn sources_list(
    ctx: &ConsoleContext,
    q: AsOfQuery,
) -> ConsoleResult<Value> {
    let historical = q.as_of_snapshot.is_some();
    let stats = record_stats_by_source(ctx).await?;
    let items = merge_source_record_stats(source_summaries(ctx).await?, &stats);
    Ok(json!({
        "items": items,
        "historicalMode": historical,
        "viewNote": historical.then_some("该视图显示实时状态"),
        "readOnly": historical,
    }))
}

pub async fn source_get(
    ctx: &ConsoleContext,
    id: String,
    _q: AsOfQuery,
) -> ConsoleResult<Value> {
    let key = normalize_source_id(&id)?;
    let stats = record_stats_by_source(ctx).await?;
    let items = merge_source_record_stats(source_summaries(ctx).await?, &stats);
    let Some(item) = items
        .into_iter()
        .find(|s| s.get("id") == Some(&Value::String(key.clone())))
    else {
        return Err(ConsoleError::not_found("信源不存在"));
    };
    let recent = recent_records_for_source(ctx, &key, 8).await?;
    let requirements = source_requirements(ctx, &key).await?;
    Ok(json!({
        "source": item,
        "recentRecords": recent,
        "requirements": requirements,
    }))
}

#[derive(Deserialize)]
pub struct TriggerBody {
    #[serde(default)]
    confirm: bool,
}

pub async fn source_trigger(
    ctx: &ConsoleContext,
    id: String,
    body: Option<TriggerBody>,
) -> ConsoleResult<Value> {
    let key = normalize_source_id(&id)?;
    if body.is_some_and(|b| !b.confirm) {
        return Err(ConsoleError::bad_request("需要 confirm: true"));
    }
    let tasks = invoke::call(&ctx.gateway, 
            "planner.trigger",
            json!({ "source": key.strip_prefix("source/").unwrap_or(&key) }),
        )
        .await
        .map_err(|e| ConsoleError::internal(e.to_string()))?;
    Ok(json!({ "tasks": tasks }))
}

#[derive(Deserialize)]
pub struct PatchSource {
    enabled: Option<bool>,
    #[serde(default)]
    confirm: bool,
}

pub async fn source_patch(
    ctx: &ConsoleContext,
    id: String,
    body: PatchSource,
) -> ConsoleResult<Value> {
    if !body.confirm {
        return Err(ConsoleError::bad_request("需要 confirm: true"));
    }
    let key = normalize_source_id(&id)?;
    let enabled = body
        .enabled
        .ok_or_else(|| ConsoleError::bad_request("需要 enabled 字段"))?;
    let rel = format!("sources/{}.yaml", key.strip_prefix("source/").unwrap_or(&key));
    let rel_path = normalize_rel_path(&rel)?;
    let path = resolve_config_file(ctx, &rel_path).await?;
    let text = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| ConsoleError::not_found(e.to_string()))?;
    write_backup(ctx, &rel, &text).await?;
    let updated = set_enabled_line(&text, enabled);
    tokio::fs::write(&path, updated)
        .await
        .map_err(|e| ConsoleError::internal(e.to_string()))?;
    validate_and_reload(ctx).await?;
    Ok(json!({ "ok": true, "enabled": enabled }))
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
    ctx: &ConsoleContext,
    q: RecordsQuery,
) -> ConsoleResult<Value> {
    let limit = q.limit.clamp(1, 100) as u32;
    let mut where_expr = None;
    if let Some(source) = &q.source {
        let sid = if source.starts_with("source/") {
            source.clone()
        } else {
            format!("source/{source}")
        };
        where_expr = Some(Expr::try_from(json!({ "eq": { "field": "sourceId", "value": sid } }))
            .map_err(|_| ConsoleError::bad_request("信源筛选无效"))?);
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
    let (rows, next_cursor) = strata_port::query(ctx, query).await?;
    let page_rows = rows;
    let needle = q.q.as_deref().map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());
    let mut items: Vec<Value> = page_rows
        .into_iter()
        .take(limit as usize)
        .map(|row| record_summary(&row))
        .collect();
    enrich_record_summaries(ctx, &mut items, as_of.clone()).await?;
    if let Some(needle) = &needle {
        items.retain(|row| record_matches_query(row, needle));
    }
    Ok(json!({
        "items": items,
        "nextCursor": next_cursor,
        "readOnly": q.as_of_snapshot.is_some(),
        "sourceOptions": source_filter_options(ctx).await?,
    }))
}

pub async fn record_get(
    ctx: &ConsoleContext,
    id: String,
    q: AsOfQuery,
) -> ConsoleResult<Value> {
    let rid_text = if id.starts_with("r_") {
        id.clone()
    } else {
        format!("r_{id}")
    };
    let as_of = q.as_of_snapshot.map(AsOf::Snapshot);
    let detail = load_record_detail(ctx, &rid_text, as_of).await?;
    Ok(detail)
}

pub async fn governance_egress(
    ctx: &ConsoleContext,
    q: AsOfQuery,
) -> ConsoleResult<Value> {
    let historical = q.as_of_snapshot.is_some();
    let egress = invoke::call(&ctx.gateway, "egress.governance_domains", json!({}))
        .await
        .map_err(|e| ConsoleError::internal(e))?;
    let proxy = egress
        .get("proxy")
        .and_then(Value::as_str)
        .unwrap_or("");
    let mut domains: Vec<Value> = egress
        .get("domains")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for row in &mut domains {
        if let Some(obj) = row.as_object_mut() {
            let state_label = obj
                .get("circuit")
                .and_then(Value::as_str)
                .unwrap_or("closed");
            obj.insert("circuitLabel".into(), json!(circuit_label(state_label)));
        }
    }
    Ok(json!({
        "domains": domains,
        "proxy": proxy,
        "historicalMode": historical,
        "viewNote": historical.then_some("该视图显示实时状态"),
        "readOnly": historical,
    }))
}

#[derive(Deserialize)]
pub struct ConfirmBody {
    #[serde(default)]
    confirm: bool,
}

pub async fn reset_circuit(
    ctx: &ConsoleContext,
    host: String,
    body: Option<ConfirmBody>,
) -> ConsoleResult<Value> {
    if body.is_some_and(|b| !b.confirm) {
        return Err(ConsoleError::bad_request("需要 confirm: true"));
    }
    invoke::call(
        &ctx.gateway,
        "egress.reset_circuit",
        json!({ "host": host }),
    )
    .await
    .map_err(|e| ConsoleError::internal(e))?;
    Ok(json!({ "ok": true, "host": host }))
}

pub async fn config_summary(ctx: &ConsoleContext) -> ConsoleResult<Value> {
    let generation = invoke::call(&ctx.gateway, "config.generation", json!({}))
        .await
        .map_err(|e| ConsoleError::internal(e.to_string()))?;
    let diff = invoke::call(&ctx.gateway, "config_inspect.diff", json!({}))
        .await
        .map_err(|e| ConsoleError::internal(e.to_string()))?;
    let diff_len = diff.as_array().map(|a| a.len()).unwrap_or(0);
    Ok(json!({
        "generation": generation.get("generation"),
        "pendingDiffCount": diff_len,
    }))
}

pub async fn config_validate(ctx: &ConsoleContext) -> ConsoleResult<Value> {
    invoke::call(&ctx.gateway, "config_inspect.validate", json!({}))
        .await
        .map_err(|e| ConsoleError::bad_request(e.to_string()))?;
    Ok(json!({ "ok": true }))
}

pub async fn config_reload(ctx: &ConsoleContext) -> ConsoleResult<Value> {
    let value = invoke::call(&ctx.gateway, "config.load", json!({}))
        .await
        .map_err(|e| ConsoleError::bad_request(e.to_string()))?;
    Ok(value)
}

pub async fn config_files_list(ctx: &ConsoleContext) -> ConsoleResult<Value> {
    let paths = list_whitelist_files(ctx).await?;
    let snapshot = context::applied(ctx).await?;
    let groups = config_file_groups(&paths, &snapshot);
    Ok(json!({ "items": paths, "groups": groups }))
}

pub async fn config_file_get(
    ctx: &ConsoleContext,
    path: String,
) -> ConsoleResult<Value> {
    let rel = normalize_rel_path(&path)?;
    let file = resolve_config_file(ctx, &rel).await?;
    let text = tokio::fs::read_to_string(&file)
        .await
        .map_err(|e| ConsoleError::not_found(e.to_string()))?;
    let has_backup = backup_path(ctx, &path).await?.is_file();
    Ok(json!({
        "path": path,
        "text": text,
        "canRollback": has_backup,
    }))
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
    ctx: &ConsoleContext,
    body: FileValidateBody,
) -> ConsoleResult<Value> {
    let rel = normalize_rel_path(&body.path)?;
    let file = resolve_config_file(ctx, &rel).await?;
    let original = if file.is_file() {
        tokio::fs::read_to_string(&file)
            .await
            .map_err(|e| ConsoleError::internal(e.to_string()))?
    } else {
        String::new()
    };
    tokio::fs::write(&file, &body.text)
        .await
        .map_err(|e| ConsoleError::internal(e.to_string()))?;
    let result = invoke::call(&ctx.gateway, "config_inspect.validate", json!({}))
        .await;
    tokio::fs::write(&file, original)
        .await
        .map_err(|e| ConsoleError::internal(e.to_string()))?;
    match result {
        Ok(_) => Ok(json!({ "ok": true })),
        Err(e) => {
            let msg = e.to_string();
            let line = validation_line_hint(&msg);
            Err(ConsoleError::bad_request(if let Some(line) = line {
                format!("第 {line} 行：{msg}")
            } else {
                msg
            }))
        }
    }
}

pub async fn config_file_put(
    ctx: &ConsoleContext,
    path: String,
    body: PutFileBody,
) -> ConsoleResult<Value> {
    if !body.confirm {
        return Err(ConsoleError::bad_request("需要 confirm: true"));
    }
    let rel = normalize_rel_path(&path)?;
    let file = resolve_config_file(ctx, &rel).await?;
    let original = if file.is_file() {
        tokio::fs::read_to_string(&file)
            .await
            .map_err(|e| ConsoleError::internal(e.to_string()))?
    } else {
        String::new()
    };
    write_backup(ctx, &path.to_string(), &original).await?;
    tokio::fs::write(&file, &body.text)
        .await
        .map_err(|e| ConsoleError::internal(e.to_string()))?;
    if let Err(e) = validate_and_reload(ctx).await {
        tokio::fs::write(&file, original)
            .await
            .map_err(|e| ConsoleError::internal(e.to_string()))?;
        return Err(e);
    }
    Ok(json!({ "ok": true, "path": path }))
}

#[derive(Deserialize)]
pub struct PutFileBody {
    text: String,
    #[serde(default)]
    confirm: bool,
}

pub async fn config_file_rollback(
    ctx: &ConsoleContext,
    body: FilePathBody,
) -> ConsoleResult<Value> {
    if !body.confirm {
        return Err(ConsoleError::bad_request("需要 confirm: true"));
    }
    let rel = normalize_rel_path(&body.path)?;
    let file = resolve_config_file(ctx, &rel).await?;
    let backup = backup_path(ctx, &body.path).await?;
    if !backup.is_file() {
        return Err(ConsoleError::not_found("没有可回滚的备份"));
    }
    let text = tokio::fs::read_to_string(&backup)
        .await
        .map_err(|e| ConsoleError::internal(e.to_string()))?;
    tokio::fs::write(&file, text)
        .await
        .map_err(|e| ConsoleError::internal(e.to_string()))?;
    validate_and_reload(ctx).await?;
    Ok(json!({ "ok": true }))
}

async fn validate_and_reload(state: &ConsoleContext) -> ConsoleResult<()> {
    invoke::call(&state.gateway, "config_inspect.validate", json!({}))
        .await
        .map_err(|e| ConsoleError::bad_request(e.to_string()))?;
    invoke::call(&state.gateway, "config.load", json!({}))
        .await
        .map_err(|e| ConsoleError::bad_request(e.to_string()))?;
    Ok(())
}

async fn backup_path(state: &ConsoleContext, rel_path: &str) -> ConsoleResult<std::path::PathBuf> {
    let safe = rel_path.replace('/', "__");
    let (data_dir, _) = context::layout(state).await?;
    Ok(data_dir.join(".console-config-backups").join(safe))
}

async fn write_backup(state: &ConsoleContext, rel_path: &str, text: &str) -> ConsoleResult<()> {
    let path = backup_path(state, rel_path).await?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ConsoleError::internal(e.to_string()))?;
    }
    std::fs::write(&path, text).map_err(|e| ConsoleError::internal(e.to_string()))?;
    Ok(())
}

fn normalize_source_id(id: &str) -> ConsoleResult<String> {
    let key = if id.starts_with("source/") {
        id.to_string()
    } else {
        format!("source/{id}")
    };
    if key.contains("..") {
        return Err(ConsoleError::bad_request("信源 ID 无效"));
    }
    Ok(key)
}

async fn source_summaries(state: &ConsoleContext) -> ConsoleResult<Vec<Value>> {
    let snapshot = context::applied(state).await?;
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
        let status = invoke::call(&state.gateway, "planner.next", json!({ "source": source_key }))
            .await
            .map_err(|e| ConsoleError::internal(e.to_string()))?;
        let health = invoke::call(&state.gateway, "health.status", json!({ "source": source_key }))
            .await
            .map_err(|e| ConsoleError::internal(e.to_string()))?;
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
            "accessorLabel": accessor_label(&accessor),
            "labels": module_labels(module),
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

async fn source_name_map(
    state: &ConsoleContext,
) -> ConsoleResult<std::collections::HashMap<String, String>> {
    let snapshot = context::applied(state).await?;
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
    Ok(out)
}

async fn source_filter_options(state: &ConsoleContext) -> ConsoleResult<Vec<Value>> {
    Ok(source_name_map(state)
        .await?
        .into_iter()
        .map(|(id, name)| json!({ "id": id, "name": name }))
        .collect())
}

async fn enrich_record_summaries(
    state: &ConsoleContext,
    items: &mut [Value],
    as_of: Option<AsOf>,
) -> ConsoleResult<()> {
    let names = source_name_map(state).await?;
    for item in items {
        let rid = item
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if rid.is_empty() {
            continue;
        }
        let parsed = strata_model::Rid::parse(&rid).map_err(|_| ConsoleError::bad_request("记录无效"))?;
        let discovery = strata_port::facet_get_at(
            state,
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
        let sid = resolve_record_source_id(state, &rid, item.get("sourceId"), &discovery).await?;
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
    state: &ConsoleContext,
    rid_text: &str,
    as_of: Option<AsOf>,
) -> ConsoleResult<Value> {
    let rid = strata_model::Rid::parse(rid_text)
        .map_err(|_| ConsoleError::bad_request("记录 ID 无效"))?;
    let discovery = strata_port::facet_get_at(
        state,
        &rid,
        FacetKind::Struct,
        Some("discovery"),
        as_of.clone(),
    )
    .await?;
    let page = strata_port::facet_get_at(
        state,
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
    let kinds = invoke::call(&state.gateway, "strata_read.kinds", json!({ "rid": rid_text }))
        .await
        .ok()
        .unwrap_or(Value::Array(vec![]));
    let names = source_name_map(state).await?;
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

struct DiscoveryFields {
    title: Option<String>,
    url: Option<String>,
    published_at: Option<String>,
    summary: Option<String>,
    author: Option<String>,
}

fn parse_discovery(facet: &Option<strata_port::FacetView>) -> DiscoveryFields {
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

fn module_labels(module: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(labels) = module.get("labels").and_then(Value::as_object) {
        for (key, value) in labels {
            if let Some(v) = value.as_str() {
                out.push(format!("{key}/{v}"));
            }
        }
    }
    out.sort();
    out
}

#[derive(Default)]
struct SourceRecordStats {
    recent_count: u64,
    last_collected_at: Option<String>,
}

async fn record_stats_by_source(state: &ConsoleContext) -> ConsoleResult<std::collections::HashMap<String, SourceRecordStats>> {
    let query = StrataQuery {
        from: QueryFrom::Resources,
        r#where: None,
        has_facets: vec![],
        expand: vec![],
        order_by: vec![],
        limit: Some(300),
        cursor: None,
        search: None,
        select: vec![],
        as_of: None,
    };
    let (rows, _) = strata_port::query(state, query).await?;
    let mut out: std::collections::HashMap<String, SourceRecordStats> = std::collections::HashMap::new();
    let names = source_name_map(state).await?;
    for row in rows {
        let rid = row.get("rid").and_then(Value::as_str).unwrap_or("");
        if rid.is_empty() {
            continue;
        }
        let parsed = strata_model::Rid::parse(rid).map_err(|_| ConsoleError::internal("记录无效"))?;
        let discovery = strata_port::facet_get_at(
            state,
            &parsed,
            FacetKind::Struct,
            Some("discovery"),
            None,
        )
        .await?;
        let sid = resolve_record_source_id(state, rid, row.get("sourceId"), &discovery).await?;
        let sid = sid.unwrap_or_default();
        if sid.is_empty() {
            continue;
        }
        let entry = out.entry(sid).or_default();
        entry.recent_count += 1;
        let updated = row.get("updatedAt").and_then(Value::as_str);
        if let Some(updated) = updated {
            let replace = entry
                .last_collected_at
                .as_ref()
                .map(|cur| updated > cur.as_str())
                .unwrap_or(true);
            if replace {
                entry.last_collected_at = Some(updated.to_string());
            }
        }
        let _ = names;
    }
    Ok(out)
}

fn merge_source_record_stats(sources: Vec<Value>, stats: &std::collections::HashMap<String, SourceRecordStats>) -> Vec<Value> {
    sources
        .into_iter()
        .map(|mut source| {
            let id = source.get("id").and_then(Value::as_str).unwrap_or("");
            if let Some(stat) = stats.get(id) {
                source["recentRecordCount"] = json!(stat.recent_count);
                if let Some(at) = &stat.last_collected_at {
                    source["lastCollectedAt"] = json!(at);
                }
            }
            source
        })
        .collect()
}

async fn resolve_record_source_id(
    state: &ConsoleContext,
    rid_text: &str,
    row_source: Option<&Value>,
    discovery: &Option<strata_port::FacetView>,
) -> ConsoleResult<Option<String>> {
    if let Some(sid) = row_source.and_then(Value::as_str).filter(|s| !s.is_empty()) {
        return Ok(Some(sid.to_string()));
    }
    if let Some(sid) = discovery
        .as_ref()
        .and_then(|f| f.value.as_ref())
        .and_then(|v| v.pointer("/sourceId"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return Ok(Some(sid.to_string()));
    }
    let related = invoke::call(&state.gateway, 
            "strata_read.related",
            json!({ "rid": rid_text, "rel": "discovered_by", "direction": "out" }),
        )
        .await
        .map_err(|e| ConsoleError::internal(e.to_string()))?;
    for edge in related.as_array().into_iter().flatten() {
        let dst = edge.get("dst").and_then(Value::as_str).unwrap_or("");
        if dst.is_empty() {
            continue;
        }
        if let Some(sid) = lookup_resource_source_id(state, dst).await? {
            return Ok(Some(sid));
        }
    }
    Ok(None)
}

async fn lookup_resource_source_id(state: &ConsoleContext, resource_rid: &str) -> ConsoleResult<Option<String>> {
    if resource_rid.starts_with("source/") {
        return Ok(Some(resource_rid.to_string()));
    }
    let rid = strata_model::Rid::parse(resource_rid).map_err(|_| ConsoleError::internal("资源无效"))?;
    let expr = Expr::try_from(json!({ "eq": { "field": "rid", "value": rid.to_string() } }))
        .map_err(|_| ConsoleError::internal("查询无效"))?;
    let query = StrataQuery {
        from: QueryFrom::Resources,
        r#where: Some(expr),
        has_facets: vec![],
        expand: vec![],
        order_by: vec![],
        limit: Some(1),
        cursor: None,
        search: None,
        select: vec![],
        as_of: None,
    };
    let (rows, _) = strata_port::query(state, query).await?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    Ok(row
        .get("sourceId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string))
}

async fn recent_records_for_source(state: &ConsoleContext, source_id: &str, limit: u32) -> ConsoleResult<Vec<Value>> {
    let where_expr = Expr::try_from(json!({ "eq": { "field": "sourceId", "value": source_id } }))
        .map_err(|_| ConsoleError::bad_request("信源筛选无效"))?;
    let query = StrataQuery {
        from: QueryFrom::Resources,
        r#where: Some(where_expr),
        has_facets: vec![],
        expand: vec![],
        order_by: vec![],
        limit: Some(limit),
        cursor: None,
        search: None,
        select: vec![],
        as_of: None,
    };
    let (rows, _) = strata_port::query(state, query).await?;
    let mut items: Vec<Value> = rows.into_iter().map(|row| record_summary(&row)).collect();
    enrich_record_summaries(state, &mut items, None).await?;
    Ok(items)
}

async fn source_requirements(state: &ConsoleContext, source_id: &str) -> ConsoleResult<Vec<Value>> {
    let snapshot = context::applied(state).await?;
    let Some(module) = snapshot.modules.get(source_id) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    let accessor = module.pointer("/access/accessor").and_then(Value::as_str).unwrap_or("");
    if accessor.contains("secret") || accessor.contains("api") {
        let has_secret = module
            .pointer("/access/secret")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty());
        if !has_secret {
            out.push(json!({
                "kind": "missing_secret",
                "label": "缺少访问密钥",
                "detail": "请在信源 YAML 中配置 access.secret 或对应密钥引用。",
            }));
        }
    }
    if !module.get("enabled").and_then(Value::as_bool).unwrap_or(true) {
        out.push(json!({
            "kind": "disabled",
            "label": "信源已停用",
            "detail": "启用后才会参与调度。",
        }));
    }
    Ok(out)
}

pub async fn dispatch(ctx: &ConsoleContext, op: &str, args: Value) -> ConsoleResult<Value> {
    match op {
        "meta" => {
            let q: AsOfQuery = serde_json::from_value(args).map_err(|_| ConsoleError::bad_request("参数无效"))?;
            meta(ctx, q).await
        }
        "timeline" => timeline(ctx).await,
        "overview" => {
            let q: AsOfQuery = serde_json::from_value(args).map_err(|_| ConsoleError::bad_request("参数无效"))?;
            overview(ctx, q).await
        }
        "sources_list" => {
            let q: AsOfQuery = serde_json::from_value(args).map_err(|_| ConsoleError::bad_request("参数无效"))?;
            sources_list(ctx, q).await
        }
        "source_get" => {
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| ConsoleError::bad_request("需要 id"))?;
            let q: AsOfQuery = serde_json::from_value(args.get("query").cloned().unwrap_or(Value::Null))
                .map_err(|_| ConsoleError::bad_request("参数无效"))?;
            source_get(ctx, id.to_string(), q).await
        }
        "source_trigger" => {
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| ConsoleError::bad_request("需要 id"))?;
            let body: Option<TriggerBody> = args
                .get("body")
                .and_then(|b| serde_json::from_value(b.clone()).ok());
            source_trigger(ctx, id.to_string(), body).await
        }
        "source_patch" => {
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| ConsoleError::bad_request("需要 id"))?;
            let body: PatchSource = serde_json::from_value(
                args.get("body").cloned().unwrap_or(Value::Null),
            )
            .map_err(|_| ConsoleError::bad_request("参数无效"))?;
            source_patch(ctx, id.to_string(), body).await
        }
        "records_list" => {
            let q: RecordsQuery = serde_json::from_value(args).map_err(|_| ConsoleError::bad_request("参数无效"))?;
            records_list(ctx, q).await
        }
        "record_get" => {
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| ConsoleError::bad_request("需要 id"))?;
            let q: AsOfQuery = serde_json::from_value(args.get("query").cloned().unwrap_or(Value::Null))
                .map_err(|_| ConsoleError::bad_request("参数无效"))?;
            record_get(ctx, id.to_string(), q).await
        }
        "governance_egress" => {
            let q: AsOfQuery = serde_json::from_value(args).map_err(|_| ConsoleError::bad_request("参数无效"))?;
            governance_egress(ctx, q).await
        }
        "reset_circuit" => {
            let host = args
                .get("host")
                .and_then(Value::as_str)
                .ok_or_else(|| ConsoleError::bad_request("需要 host"))?;
            let body: Option<ConfirmBody> = args
                .get("body")
                .and_then(|b| serde_json::from_value(b.clone()).ok());
            reset_circuit(ctx, host.to_string(), body).await
        }
        "config_summary" => config_summary(ctx).await,
        "config_validate" => config_validate(ctx).await,
        "config_reload" => config_reload(ctx).await,
        "config_files_list" => config_files_list(ctx).await,
        "config_file_get" => {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| ConsoleError::bad_request("需要 path"))?;
            config_file_get(ctx, path.to_string()).await
        }
        "config_file_validate_text" => {
            let body: FileValidateBody = serde_json::from_value(args).map_err(|_| ConsoleError::bad_request("参数无效"))?;
            config_file_validate_text(ctx, body).await
        }
        "config_file_put" => {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| ConsoleError::bad_request("需要 path"))?;
            let body: PutFileBody = serde_json::from_value(
                args.get("body").cloned().unwrap_or(Value::Null),
            )
            .map_err(|_| ConsoleError::bad_request("参数无效"))?;
            config_file_put(ctx, path.to_string(), body).await
        }
        "config_file_rollback" => {
            let body: FilePathBody = serde_json::from_value(args).map_err(|_| ConsoleError::bad_request("参数无效"))?;
            config_file_rollback(ctx, body).await
        }
        other => Err(ConsoleError::not_found(format!("未知操作：{other}"))),
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
