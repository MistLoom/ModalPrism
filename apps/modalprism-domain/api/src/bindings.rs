//! AdapterBinding 配置面 REST（R6）。
//!
//! 提供绑定的 list / get / create / update / delete，对齐既有 catalog 配置 API 范式：
//! 复用 etag / `If-Match` 并发控制（R5.6）、写前经 `config::create/update/remove` 的整库
//! 校验 + 失败回滚（R10.1），删除守卫由 `config::remove` 的引用检查兜底（R5.2）。
//! 生命周期里 `enabled:false` 只停用保留绑定，硬删才走级联（R5.7，由 disable/enable 表达）。

use axum::Json;
use axum::Router;
use axum::extract::{Path as AxPath, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::get;
use serde_json::{Map, Value, json};

use crate::catalog::ListQuery;
use crate::error::ApiError;
use crate::observe::Refs;
use crate::util::{self, cursor_offset, page};
use crate::{AppState, Shared};

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/catalog/bindings", get(list_bindings).post(create_binding))
        .route(
            "/catalog/bindings/{bindingId}",
            get(get_binding)
                .patch(update_binding)
                .delete(delete_binding),
        )
        .route(
            "/catalog/sources/{sourceId}/bindings",
            axum::routing::patch(rebind_source_bindings),
        )
}

fn binding_etag(doc: &Value) -> String {
    util::etag(&["binding", &util::hash_short(&doc.to_string())])
}

fn public_row(entry: &Value) -> Value {
    json!({
        "id": entry["slug"],
        "bindingId": entry["id"],
        "name": entry["name"],
        "description": entry["description"],
        "enabled": entry["enabled"],
        "adapter": entry["adapter"],
        "params": entry["params"],
        "referencedBy": entry["referencedBy"],
        "references": entry["references"],
        "removable": entry["removable"],
        "etag": binding_etag(&entry["doc"]),
    })
}

async fn binding_rows(state: &AppState) -> Result<Vec<Value>, ApiError> {
    let list = config::list(&state.config_dir)?;
    Ok(list["bindings"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|entry| public_row(&entry))
        .collect())
}

async fn list_bindings(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let rows = binding_rows(&state).await?;
    let limit = util::limit(query.limit.as_deref())?;
    let filter = format!("bindings:{}", query.query.clone().unwrap_or_default());
    let offset = cursor_offset(query.cursor.as_deref(), &filter)?;
    let needle = query.query.unwrap_or_default().to_lowercase();
    let matched: Vec<Value> = rows
        .into_iter()
        .filter(|row| {
            needle.is_empty()
                || row["id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_lowercase()
                    .contains(&needle)
                || row["adapter"]
                    .as_str()
                    .unwrap_or_default()
                    .to_lowercase()
                    .contains(&needle)
        })
        .collect();
    let total = matched.len();
    let items: Vec<Value> = matched.into_iter().skip(offset).take(limit + 1).collect();
    let more = items.len() > limit;
    let items: Vec<Value> = items.into_iter().take(limit).collect();
    let next = if more || offset + items.len() < total {
        Some(util::cursor(&filter, offset + limit))
    } else {
        None
    };
    Ok(util::ok(&page(items, next)))
}

async fn get_binding(
    State(state): State<Shared>,
    AxPath(binding_id): AxPath<String>,
) -> Result<Response, ApiError> {
    let row = binding_by_slug(&state, &binding_id).await?;
    let mut response = util::ok(&row);
    if let Ok(value) = row["etag"].as_str().unwrap_or("").parse() {
        response
            .headers_mut()
            .insert(axum::http::header::ETAG, value);
    }
    Ok(response)
}

/// POST 体：`{name, adapter, params?, endpoint?, urlTemplate?, selector?, envRefs?, description?, enabled?}`。
const BINDING_FIELDS: &[&str] = &[
    "name",
    "adapter",
    "params",
    "endpoint",
    "urlTemplate",
    "selector",
    "envRefs",
    "description",
    "enabled",
];

fn binding_doc(map: &mut serde_json::Map<String, Value>) -> Result<Value, ApiError> {
    let adapter = util::non_empty(map, "adapter")?;
    let mut doc = serde_json::Map::new();
    doc.insert(
        "name".into(),
        map.get("name").cloned().unwrap_or(json!(adapter)),
    );
    doc.insert(
        "adapter".into(),
        Value::String(if adapter.starts_with("adapter/") {
            adapter
        } else {
            format!("adapter/{adapter}")
        }),
    );
    if let Some(params) = util::optional_object(map, "params")? {
        doc.insert("params".into(), Value::Object(params));
    }
    for key in ["endpoint", "urlTemplate"] {
        if let Some(value) = util::optional_text(map, key)? {
            doc.insert(key.into(), Value::String(value));
        }
    }
    if let Some(selector) = util::optional_object(map, "selector")? {
        doc.insert("selector".into(), Value::Object(selector));
    }
    if let Some(env_refs) = util::optional_object(map, "envRefs")? {
        doc.insert("envRefs".into(), Value::Object(env_refs));
    }
    if let Some(description) = map.get("description") {
        doc.insert("description".into(), description.clone());
    }
    if let Some(enabled) = map.get("enabled") {
        doc.insert("enabled".into(), enabled.clone());
    }
    Ok(Value::Object(doc))
}

async fn create_binding(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let mut map = util::object(&body)?.clone();
    util::unknown(&map, BINDING_FIELDS)?;
    let doc = binding_doc(&mut map)?;
    let name = doc["name"].as_str().unwrap_or("binding").to_string();
    let base = util::slug(&name);
    let mut slug = base.clone();
    for index in 2..20 {
        if !std::path::Path::new(&state.config_dir)
            .join("bindings")
            .join(format!("{slug}.yaml"))
            .exists()
        {
            break;
        }
        slug = format!("{base}-{index}");
    }
    let saved = config::create(
        &state.config_dir,
        &crate::catalog::options(),
        &config::Draft {
            kind: "binding".into(),
            slug: slug.clone(),
            doc,
        },
    )?;
    state.services.config_reload()?;
    crate::observe::append_audit(
        &state,
        "catalog.binding.create",
        "succeeded",
        "新建绑定",
        json!({"id": saved.id, "name": name}),
        None,
    )
    .await?;
    let row = binding_by_slug(&state, &slug).await?;
    Ok(util::created(&row))
}

async fn update_binding(
    State(state): State<Shared>,
    AxPath(binding_id): AxPath<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let mut map = util::object(&body)?.clone();
    util::unknown(&map, BINDING_FIELDS)?;
    let current = binding_by_slug(&state, &binding_id).await?;
    util::if_match(&headers, current["etag"].as_str().unwrap_or(""))?;
    let doc = binding_doc(&mut map)?;
    config::update(
        &state.config_dir,
        &crate::catalog::options(),
        &config::Draft {
            kind: "binding".into(),
            slug: binding_id.clone(),
            doc,
        },
    )?;
    state.services.config_reload()?;
    crate::observe::append_audit(
        &state,
        "catalog.binding.update",
        "succeeded",
        "修改绑定",
        json!({"id": binding_id}),
        None,
    )
    .await?;
    let row = binding_by_slug(&state, &binding_id).await?;
    Ok(util::ok(&row))
}

async fn delete_binding(
    State(state): State<Shared>,
    AxPath(binding_id): AxPath<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let current = binding_by_slug(&state, &binding_id).await?;
    util::if_match(&headers, current["etag"].as_str().unwrap_or(""))?;
    // R5.2 删除守卫：仍被引用时 `config::remove` 会拒删并回显占用方，映射为 412。
    config::remove(
        &state.config_dir,
        &crate::catalog::options(),
        "binding",
        &binding_id,
    )
    .map_err(|error| {
        let message = error.to_string();
        if message.contains("仍被信源引用") {
            ApiError::precondition(message)
        } else {
            ApiError::from(error)
        }
    })?;
    state.services.config_reload()?;
    crate::observe::append_audit(
        &state,
        "catalog.binding.delete",
        "succeeded",
        "删除绑定",
        json!({"id": binding_id}),
        None,
    )
    .await?;
    Ok(util::ok(
        &json!({"partial": false, "bindingId": binding_id, "deleted": true}),
    ))
}

async fn binding_by_slug(state: &AppState, slug: &str) -> Result<Value, ApiError> {
    binding_rows(state)
        .await?
        .into_iter()
        .find(|row| row["id"] == slug)
        .ok_or_else(|| ApiError::not_found(format!("绑定 {slug}")))
}

/// 换绑作业入口（R5.4）：把「先 binding 后 source」的多文件变更收敛成单个 Operation。
///
/// body：`{bindings: [{slug?, adapter, params?, endpoint?, urlTemplate?, selector?, envRefs?, description?, when?, for?, after?}]}`。
/// 要求 `If-Match`（R5.6）。异步返回 202 + Operation，后台 [`execute_binding_rebind`] 落盘。
async fn rebind_source_bindings(
    State(state): State<Shared>,
    AxPath(source_id): AxPath<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["bindings"])?;
    let bindings = map.get("bindings").cloned().unwrap_or(Value::Null);
    if !bindings.is_array() {
        return Err(ApiError::validation("bindings", "type", "必须是数组"));
    }
    let source_entry = config::entries(&state.config_dir, "source")?
        .into_iter()
        .find(|entry| entry.slug == source_id)
        .ok_or_else(|| ApiError::not_found(format!("信源 {source_id}")))?;
    let etag = util::etag(&[&source_id, &util::hash_short(&source_entry.doc.to_string())]);
    util::if_match(&headers, &etag)?;
    let operation = crate::operations::start(
        &state,
        "binding.rebind",
        vec![json!({"kind": "source", "id": source_id})],
        json!({"kind": "binding.rebind", "sourceId": source_id, "bindings": bindings}),
    )
    .await?;
    Ok(crate::operations::accepted(&operation))
}

/// 换绑作业的后台执行：构建写入集 → 原子落盘（失败全回滚）→ 热重载 → 审计。
pub async fn execute_binding_rebind(
    state: &AppState,
    operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let source_id = action["sourceId"].as_str().unwrap_or_default();
    let desired = action["bindings"].as_array().cloned().unwrap_or_default();
    let entries = config::entries(&state.config_dir, "source")?;
    let current = entries
        .iter()
        .find(|entry| entry.slug == source_id)
        .map(|entry| (entry.file.clone(), entry.doc.clone()))
        .ok_or_else(|| ApiError::not_found(format!("信源 {source_id}")))?;
    let others: Vec<Value> = entries
        .iter()
        .filter(|entry| entry.slug != source_id)
        .map(|entry| entry.doc.clone())
        .collect();

    let (new_source_doc, binding_writes, remove_slugs) =
        build_rebind(source_id, &current.1, &desired, &others)?;

    let mut writes: Vec<config::AtomicWrite> = Vec::new();
    // 先 binding 后 source（R5.4）。
    for (slug, doc) in &binding_writes {
        writes.push(config::AtomicWrite::write(
            format!("bindings/{slug}.yaml"),
            config::to_yaml(doc),
        ));
    }
    writes.push(config::AtomicWrite::write(
        current.0.clone(),
        config::to_yaml(&new_source_doc),
    ));
    for slug in &remove_slugs {
        writes.push(config::AtomicWrite::remove(format!("bindings/{slug}.yaml")));
    }
    config::apply_atomic(&state.config_dir, &crate::catalog::options(), &writes)?;
    state.services.config_reload()?;
    crate::observe::append_audit(
        state,
        "catalog.binding.rebind",
        "succeeded",
        "换绑信源",
        json!({
            "sourceId": source_id,
            "bindingsWritten": binding_writes.len(),
            "bindingsRemoved": remove_slugs.len(),
        }),
        Some(Refs::op(operation_id)),
    )
    .await?;
    Ok(json!({
        "partial": false,
        "sourceId": source_id,
        "bindingsWritten": binding_writes.len(),
        "bindingsRemoved": remove_slugs.len(),
    }))
}

/// 换绑写入计划：(新信源正文, 待写绑定 `[(slug, doc)]`, 待删孤儿 slug)。
type RebindPlan = (Value, Vec<(String, Value)>, Vec<String>);

/// 纯函数：由期望绑定集推导新信源正文、待写绑定、待删孤儿绑定（R5.3/R5.5 换绑后旧引用归零）。
///
/// 不触盘，便于单测。`others` 是除本信源外的全部信源正文，用于判定旧绑定是否变成孤儿。
fn build_rebind(
    source_slug: &str,
    current_source: &Value,
    desired: &[Value],
    others: &[Value],
) -> Result<RebindPlan, ApiError> {
    let old_refs = binding_refs_in(current_source);
    let mut binding_writes: Vec<(String, Value)> = Vec::new();
    let mut new_pipeline: Vec<Value> = Vec::new();
    let mut new_ids: Vec<String> = Vec::new();
    for (index, spec) in desired.iter().enumerate() {
        let map = spec
            .as_object()
            .ok_or_else(|| ApiError::validation("bindings", "type", "绑定项必须是对象"))?;
        let adapter = util::text(map, "adapter")?;
        let adapter = if adapter.starts_with("adapter/") {
            adapter
        } else {
            format!("adapter/{adapter}")
        };
        let slug = match util::optional_text(map, "slug")? {
            Some(value) if !value.trim().is_empty() => value,
            _ => format!(
                "{source_slug}-{index}-{}",
                adapter.rsplit('/').next().unwrap_or("adapter")
            ),
        };
        let binding_id = format!("binding/{slug}");
        let mut doc = Map::new();
        doc.insert("id".into(), Value::String(binding_id.clone()));
        doc.insert("kind".into(), Value::String("binding".into()));
        doc.insert(
            "name".into(),
            map.get("name").cloned().unwrap_or(json!(slug)),
        );
        doc.insert("adapter".into(), Value::String(adapter));
        if let Some(params) = map.get("params").filter(|v| v.is_object()) {
            doc.insert("params".into(), params.clone());
        }
        for key in [
            "endpoint",
            "urlTemplate",
            "selector",
            "envRefs",
            "description",
            "enabled",
        ] {
            if let Some(value) = map.get(key) {
                doc.insert(key.into(), value.clone());
            }
        }
        binding_writes.push((slug, Value::Object(doc)));
        new_ids.push(binding_id.clone());
        let mut step = Map::new();
        for key in ["when", "for", "after"] {
            if let Some(value) = map.get(key) {
                step.insert(key.into(), value.clone());
            }
        }
        step.insert("binding".into(), Value::String(binding_id));
        new_pipeline.push(Value::Object(step));
    }
    let mut new_source_doc = current_source.clone();
    new_source_doc["pipeline"] = Value::Array(new_pipeline);
    // 旧绑定里：本信源不再引用、且无其它信源引用 ⇒ 孤儿，删除（共享绑定保留）。
    let remove_slugs: Vec<String> = old_refs
        .into_iter()
        .filter(|id| !new_ids.contains(id))
        .filter(|id| !others.iter().any(|doc| mentions(doc, id)))
        .filter_map(|id| id.split_once('/').map(|(_, stem)| stem.to_string()))
        .collect();
    Ok((new_source_doc, binding_writes, remove_slugs))
}

/// 递归收集正文里的 `binding/...` 引用。
fn binding_refs_in(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    collect(value, &mut out);
    out.sort();
    out.dedup();
    out
}

fn collect(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            if text.starts_with("binding/") {
                out.push(text.clone());
            }
        }
        Value::Array(items) => items.iter().for_each(|item| collect(item, out)),
        Value::Object(map) => map.values().for_each(|item| collect(item, out)),
        _ => {}
    }
}

/// 某文档是否提到给定 id。
fn mentions(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(text) => text == needle,
        Value::Array(items) => items.iter().any(|item| mentions(item, needle)),
        Value::Object(map) => map.values().any(|item| mentions(item, needle)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rebind_writes_bindings_rewrites_pipeline_and_reclaims_orphans() {
        let current = json!({
            "id": "source/demo",
            "kind": "source",
            "name": "Demo",
            "access": {"accessor": "accessor/rss", "locator": {"url": "https://e.com/f"}},
            "pipeline": [{"binding": "binding/demo-old"}],
            "schedule": {"interval": "1h"},
            "admission": {}
        });
        let desired = vec![
            json!({"slug": "b1", "adapter": "json-list", "params": {"items": "/data"}, "for": "follow"}),
        ];
        let (new_src, writes, removes) = build_rebind("demo", &current, &desired, &[]).unwrap();
        // 信源 pipeline 改为引用新绑定，保留 for。
        assert_eq!(new_src["pipeline"][0]["binding"], json!("binding/b1"));
        assert_eq!(new_src["pipeline"][0]["for"], json!("follow"));
        // 新绑定文档形状正确，adapter 归一化。
        assert_eq!(writes[0].0, "b1");
        assert_eq!(writes[0].1["id"], json!("binding/b1"));
        assert_eq!(writes[0].1["adapter"], json!("adapter/json-list"));
        assert_eq!(writes[0].1["params"]["items"], json!("/data"));
        // 旧绑定无人引用 ⇒ 孤儿回收。
        assert_eq!(removes, vec!["demo-old".to_string()]);
    }

    #[test]
    fn rebind_retains_binding_shared_with_other_source() {
        let current = json!({"id": "source/a", "pipeline": [{"binding": "binding/shared"}]});
        let desired = vec![json!({"slug": "a2", "adapter": "json-list"})];
        let others = vec![json!({"id": "source/b", "pipeline": [{"binding": "binding/shared"}]})];
        let (_, _, removes) = build_rebind("a", &current, &desired, &others).unwrap();
        assert!(removes.is_empty(), "共享绑定不应被回收");
    }

    #[test]
    fn binding_refs_in_collects_deduped() {
        let doc = json!({"pipeline": [{"binding": "binding/x"}, {"binding": "binding/y"}, {"binding": "binding/x"}]});
        assert_eq!(
            binding_refs_in(&doc),
            vec!["binding/x".to_string(), "binding/y".to_string()]
        );
    }
}
