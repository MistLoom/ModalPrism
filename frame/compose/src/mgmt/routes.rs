//! Mgmt API 路由与状态。

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use ability_core::AbilityHost;
use ability_support::{
    DiscoveryPayload, EventClass, EventFilter, EventListener, InstanceId, LoadSpec, MethodId,
    SourceId, load_ability_package,
};
use async_stream::stream;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use frame_core::{AbilityExpose, Error, ErrorKind, Gateway, InvokeRequest, LocalsScope};
use futures_core::stream::Stream;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::bootstrap::AppHandle;
use crate::catalog::{WorkspaceCatalog, resolve_protocols};
use crate::compose::{apply_expose_delta, reload_one};
use crate::load_order::{suggest_enable, unload_impact, write_expose_file};

/// 管理面共享状态。
#[derive(Clone)]
pub struct MgmtState {
    pub host: AbilityHost,
    pub gateway: Arc<dyn Gateway>,
    pub expose: Arc<RwLock<AbilityExpose>>,
    pub catalog: WorkspaceCatalog,
    pub config_path: Option<PathBuf>,
    pub invoke_log: Arc<RwLock<Vec<Value>>>,
}

impl From<AppHandle> for MgmtState {
    fn from(app: AppHandle) -> Self {
        Self {
            host: app.host,
            gateway: app.gateway,
            expose: Arc::new(RwLock::new(app.expose)),
            catalog: app.catalog,
            config_path: app.config_path,
            invoke_log: app.invoke_log,
        }
    }
}

pub fn router(state: MgmtState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/runtime", get(runtime_state))
        .route("/api/graph", get(graph))
        .route("/api/expose", get(get_expose).put(put_expose))
        .route("/api/assembly/suggest-enable", post(suggest_enable_api))
        .route("/api/assembly/unload-impact", post(unload_impact_api))
        .route("/api/instances/reload", post(reload_instance))
        .route("/api/instances/reload-all", post(reload_all_instances))
        .route("/api/observability/chains", get(obs_chains))
        .route("/api/observability/handlers", get(obs_handlers))
        .route("/api/observability/invokes", get(obs_invokes))
        .route("/api/load", post(load_instance))
        .route("/api/unload", post(unload_instance))
        .route("/api/invoke", post(invoke))
        .route("/api/events", get(events))
        .with_state(state)
}

async fn health(State(state): State<MgmtState>) -> Json<Value> {
    let catalog = state.host.catalog();
    let unique: HashSet<_> = catalog.iter().map(|e| e.instance.as_str()).collect();
    Json(json!({
        "status": "ok",
        "instances": unique.len(),
        "methods": catalog.len(),
        "registered": state.catalog.registered_count(),
    }))
}

#[derive(serde::Serialize)]
struct ResolvedExpose {
    id: String,
    enabled: bool,
    config: Value,
    loaded: bool,
    phase: Option<String>,
}

async fn runtime_state(State(state): State<MgmtState>) -> Json<Value> {
    let catalog = state.host.catalog();
    let instances: Vec<Value> = state
        .host
        .instances_snapshot()
        .into_iter()
        .map(|d| serde_json::to_value(d).unwrap_or(Value::Null))
        .collect();

    let protocols: Vec<Value> = state
        .catalog
        .sources()
        .iter()
        .map(|entry| {
            let kinds = resolve_protocols(&entry.root);
            let loaded = catalog.iter().any(|c| c.instance.as_str() == entry.id);
            json!({
                "id": entry.id,
                "protocols": kinds.iter().map(|k| k.as_str()).collect::<Vec<_>>(),
                "loaded": loaded,
                "linked": entry.linked,
            })
        })
        .collect();

    let resolved: Vec<ResolvedExpose> = state
        .catalog
        .sources()
        .iter()
        .map(|entry| {
            let r = state.expose.read().expect("expose").resolve(&entry.id);
            let phase = catalog
                .iter()
                .find(|c| c.instance.as_str() == entry.id)
                .map(|c| format!("{:?}", c.phase));
            ResolvedExpose {
                id: entry.id.clone(),
                enabled: r.enabled,
                config: r.config,
                loaded: phase.is_some(),
                phase,
            }
        })
        .collect();

    Json(json!({
        "instances": instances,
        "catalog": catalog,
        "expose": state.expose.read().expect("expose").clone(),
        "resolved": resolved,
        "protocols": protocols,
    }))
}

async fn graph(State(state): State<MgmtState>) -> Json<Value> {
    let loaded: HashSet<String> = state
        .host
        .catalog()
        .iter()
        .map(|e| e.instance.to_string())
        .collect();

    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    for entry in state.catalog.sources() {
        let (stateful, depends) = match load_ability_package(&entry.root) {
            Ok(pkg) => {
                let deps: Vec<Value> = pkg
                    .decl
                    .depends
                    .iter()
                    .filter_map(|d| serde_json::to_value(d).ok())
                    .collect();
                (pkg.decl.stateful, deps)
            }
            Err(_) => (false, vec![]),
        };
        let resolved = state.expose.read().expect("expose").resolve(&entry.id);
        nodes.push(json!({
            "id": entry.id,
            "stateful": stateful,
            "enabled": resolved.enabled,
            "loaded": loaded.contains(&entry.id),
            "linked": entry.linked,
        }));
        for dep in depends {
            let target = dep
                .get("id")
                .and_then(|v| v.as_str())
                .or_else(|| dep.as_str())
                .unwrap_or("");
            if !target.is_empty() {
                edges.push(json!({
                    "from": entry.id,
                    "to": target,
                    "optional": dep.get("optional").and_then(|v| v.as_bool()).unwrap_or(false),
                }));
            }
        }
    }

    Json(json!({ "nodes": nodes, "edges": edges }))
}

async fn get_expose(State(state): State<MgmtState>) -> Json<Value> {
    Json(json!({
        "expose": state.expose.read().expect("expose").clone(),
        "config_path": state.config_path.as_ref().map(|p| p.display().to_string()),
    }))
}

#[derive(Deserialize)]
struct ExposeBody {
    expose: AbilityExpose,
    #[serde(default)]
    apply: bool,
    #[serde(default)]
    save_file: bool,
}

async fn put_expose(
    State(state): State<MgmtState>,
    Json(body): Json<ExposeBody>,
) -> Result<Json<Value>, ApiError> {
    let old = state.expose.read().expect("expose").clone();
    *state.expose.write().expect("expose") = body.expose.clone();
    state.host.set_expose(body.expose.clone());
    if body.apply {
        apply_expose_delta(&state.host, &state.catalog, &old, &body.expose)
            .await
            .map_err(ApiError)?;
    }
    let mut saved_path = None;
    if body.save_file {
        if let Some(path) = &state.config_path {
            write_expose_file(path, &body.expose).map_err(|e| Error::invalid_args(e))?;
            saved_path = Some(path.display().to_string());
        } else {
            return Err(Error::invalid_args("未指定 --config，无法写回文件").into());
        }
    }
    Ok(Json(json!({
        "ok": true,
        "applied": body.apply,
        "saved_path": saved_path,
    })))
}

#[derive(Deserialize)]
struct SuggestEnableBody {
    targets: Vec<String>,
    #[serde(default)]
    enabled: BTreeMap<String, bool>,
}

async fn suggest_enable_api(
    State(state): State<MgmtState>,
    Json(body): Json<SuggestEnableBody>,
) -> Json<Value> {
    let expose = state.expose.read().expect("expose").clone();
    let mut draft = expose.clone();
    for (id, on) in &body.enabled {
        draft.abilities.entry(id.clone()).or_default().enabled = Some(*on);
    }
    let result = suggest_enable(&state.catalog, &body.targets, &draft);
    Json(json!(result))
}

#[derive(Deserialize)]
struct UnloadImpactBody {
    instance: String,
}

async fn unload_impact_api(
    State(state): State<MgmtState>,
    Json(body): Json<UnloadImpactBody>,
) -> Json<Value> {
    let loaded: HashSet<String> = state
        .host
        .catalog()
        .iter()
        .map(|e| e.instance.to_string())
        .collect();
    let expose = state.expose.read().expect("expose").clone();
    let impact = unload_impact(&state.catalog, &body.instance, &loaded, &expose);
    Json(json!(impact))
}

#[derive(Deserialize)]
struct ReloadBody {
    instance: String,
}

async fn reload_instance(
    State(state): State<MgmtState>,
    Json(body): Json<ReloadBody>,
) -> Result<Json<Value>, ApiError> {
    let expose = state.expose.read().expect("expose").clone();
    reload_one(&state.host, &state.catalog, &expose, &body.instance)
        .await
        .map_err(ApiError)?;
    Ok(Json(json!({ "reloaded": body.instance })))
}

async fn reload_all_instances(State(state): State<MgmtState>) -> Result<Json<Value>, ApiError> {
    let expose = state.expose.read().expect("expose").clone();
    let handle = AppHandle {
        host: state.host.clone(),
        gateway: Arc::clone(&state.gateway),
        expose: expose.clone(),
        catalog: state.catalog.clone(),
        config_path: state.config_path.clone(),
        invoke_log: Arc::clone(&state.invoke_log),
        _host_event_regs: vec![],
    };
    let ids = handle.reload_all(&expose).await.map_err(ApiError)?;
    Ok(Json(json!({ "reloaded": ids })))
}

async fn obs_chains(State(state): State<MgmtState>) -> Json<Value> {
    Json(json!({ "chains": state.host.chains_snapshot() }))
}

async fn obs_handlers(State(state): State<MgmtState>) -> Json<Value> {
    Json(json!({
        "handlers": state.host.events().registrations_snapshot(),
        "user_sinks": state.host.events().user_sink_count(),
    }))
}

async fn obs_invokes(State(state): State<MgmtState>) -> Json<Value> {
    let log = state.invoke_log.read().expect("invoke_log").clone();
    Json(json!({ "entries": log }))
}

#[derive(Deserialize)]
struct LoadBody {
    source: String,
    #[serde(rename = "as")]
    as_name: String,
    #[serde(default)]
    config: Value,
}

async fn load_instance(
    State(state): State<MgmtState>,
    Json(body): Json<LoadBody>,
) -> Result<Json<Value>, ApiError> {
    let spec = LoadSpec::new(
        SourceId::new(&body.source).map_err(|e| Error::invalid_args(e.to_string()))?,
        InstanceId::new(&body.as_name).map_err(|e| Error::invalid_args(e.to_string()))?,
    )
    .with_config(body.config);
    let id = state.host.load(spec).await.map_err(ApiError)?;
    Ok(Json(json!({ "instance": id.to_string() })))
}

#[derive(Deserialize)]
struct UnloadBody {
    instance: String,
}

async fn unload_instance(
    State(state): State<MgmtState>,
    Json(body): Json<UnloadBody>,
) -> Result<Json<Value>, ApiError> {
    let id = InstanceId::new(&body.instance).map_err(|e| Error::invalid_args(e.to_string()))?;
    state.host.unload(&id).await.map_err(ApiError)?;
    Ok(Json(json!({ "unloaded": id.to_string() })))
}

#[derive(Deserialize)]
struct InvokeBody {
    instance: String,
    method: String,
    #[serde(default)]
    args: Value,
    #[serde(default)]
    session_id: Option<String>,
}

async fn invoke(
    State(state): State<MgmtState>,
    Json(body): Json<InvokeBody>,
) -> Result<Json<Value>, ApiError> {
    let inst = InstanceId::new(&body.instance).map_err(|e| Error::invalid_args(e.to_string()))?;
    let method = MethodId::new(&body.method).map_err(|e| Error::invalid_args(e.to_string()))?;
    let mut req = InvokeRequest::new(inst, method, body.args);
    if let Some(sid) = body.session_id.filter(|s| !s.is_empty()) {
        let scope = sid
            .parse::<LocalsScope>()
            .map_err(|e| Error::invalid_args(e.to_string()))?;
        req = req.with_locals(scope);
    }
    let resp = state.gateway.invoke(req).await.map_err(ApiError)?;
    Ok(Json(json!({
        "data": resp.data,
        "session_id": resp.locals_scope.map(|s| s.to_string()),
    })))
}

async fn events(
    State(state): State<MgmtState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>> + Send> {
    let mut listener = state.host.events().listen(EventFilter::default()).await;
    let s = stream! {
        loop {
            match listener.next().await {
                Some(ev) => {
                    let data = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
                    yield Ok(Event::default().data(data));
                }
                None => break,
            }
        }
    };
    Sse::new(s).keep_alive(KeepAlive::default())
}

struct ApiError(Error);

impl From<Error> for ApiError {
    fn from(value: Error) -> Self {
        Self(value)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.0.kind() {
            ErrorKind::NotFound | ErrorKind::SessionNotFound => StatusCode::NOT_FOUND,
            ErrorKind::InvalidInput | ErrorKind::SessionRequired | ErrorKind::Config => {
                StatusCode::BAD_REQUEST
            }
            ErrorKind::Failed
            | ErrorKind::Dependency
            | ErrorKind::Unavailable
            | ErrorKind::Upstream => StatusCode::UNPROCESSABLE_ENTITY,
            ErrorKind::Io => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = Json(json!({
            "error": { "type": self.0.code(), "message": self.0.to_string() }
        }));
        (status, body).into_response()
    }
}
