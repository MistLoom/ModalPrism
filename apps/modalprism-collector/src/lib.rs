//! 采集进程组合根：启动参数、Ability 装配、运维 HTTP 与后台循环（经 Gateway）。

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ability_core::{AbilityGrant, AbilityHost};
use ability_support::{InstanceId, LoadSpec, MethodId, SourceId};
use collector_runtime::{Binds, Error, refuse_remote, require_loopback};
use config::LoadOptions;
use frame_core::{AbilityExpose, AccessKind, Gateway, InvokeRequest};
use frame_port_http::HttpPort;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

mod console_static;

pub struct Running {
    host: Arc<AbilityHost>,
    gateway: Arc<dyn Gateway>,
    http: SocketAddr,
    proxy: SocketAddr,
    push: SocketAddr,
    stop: Arc<AtomicBool>,
    tasks: Vec<JoinHandle<()>>,
}

impl Running {
    pub fn host(&self) -> &AbilityHost {
        &self.host
    }

    pub fn gateway(&self) -> Arc<dyn Gateway> {
        Arc::clone(&self.gateway)
    }

    pub fn http(&self) -> SocketAddr {
        self.http
    }

    pub fn proxy(&self) -> SocketAddr {
        self.proxy
    }

    pub fn push(&self) -> SocketAddr {
        self.push
    }

    pub async fn shutdown(self) -> Result<(), Error> {
        self.stop.store(true, Ordering::Relaxed);
        for task in &self.tasks {
            task.abort();
        }
        let _ = invoke_host(&self.host, "planner", "shutdown", json!({})).await;
        Ok(())
    }
}

pub async fn boot(data: &Path, config_dir: &Path, binds: Binds) -> Result<Running, Error> {
    let load_options = LoadOptions::from_process();
    let snapshot = config::validate(config_dir, &load_options)?;
    refuse_remote(&snapshot, data)?;
    require_loopback(binds.http)?;
    require_loopback(binds.proxy)?;
    require_loopback(binds.push)?;

    let data_dir = collector_runtime::normalize(data);
    let http = reserve_addr(binds.http).await?;
    let proxy = reserve_addr(binds.proxy).await?;
    let push = reserve_addr(binds.push).await?;
    let proxy_url = format!("http://{proxy}");
    let browser = std::env::var("MODALPRISM_MOLI").unwrap_or_else(|_| "moli".into());

    let bind = json!({
        "dataDir": data_dir,
        "configDir": config_dir,
        "system": snapshot.system,
        "proxyUrl": proxy_url,
        "browserProgram": browser,
        "proxyListen": proxy.to_string(),
        "pushListen": push.to_string(),
    });
    let strata_config = json!({
        "dataDir": data_dir,
        "system": snapshot.system,
    });

    let host = link(bind, strata_config).await?;
    let gateway: Arc<dyn Gateway> = host.as_ref().clone().into_gateway();
    let stop = Arc::new(AtomicBool::new(false));

    let _ = invoke_host(&host, "committer", "maintain", json!({})).await;

    let http_gateway = Arc::clone(&gateway);
    let http_task = tokio::spawn(async move {
        serve_ops(http, http_gateway).await;
    });
    wait_http(http).await?;

    let tasks = vec![
        http_task,
        spawn_planner_loop(Arc::clone(&host), Arc::clone(&stop)),
        spawn_flush_loop(Arc::clone(&host), Arc::clone(&stop)),
        spawn_health_loop(Arc::clone(&host), Arc::clone(&stop)),
    ];

    Ok(Running {
        host,
        gateway,
        http,
        proxy,
        push,
        stop,
        tasks,
    })
}

async fn reserve_addr(addr: SocketAddr) -> Result<SocketAddr, Error> {
    if addr.port() != 0 {
        return Ok(addr);
    }
    let probe = TcpListener::bind(addr)
        .await
        .map_err(|source| Error::with_source("监听", source))?;
    let local = probe
        .local_addr()
        .map_err(|source| Error::with_source("监听", source))?;
    drop(probe);
    Ok(local)
}

async fn wait_http(addr: SocketAddr) -> Result<(), Error> {
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Err(Error::with_source(
        "监听",
        std::io::Error::other("监听"),
    ))
}

fn ops_expose() -> AbilityExpose {
    AbilityExpose::from_value(&json!({
        "abilities": {
            "strata_read": { "enabled": true },
            "planner": { "enabled": true },
            "health": { "enabled": true },
            "config_inspect": { "enabled": true }
        }
    }))
    .expect("expose")
}

async fn link(bind: Value, strata_config: Value) -> Result<Arc<AbilityHost>, Error> {
    let host = Arc::new(AbilityHost::new());
    host.set_entry_app("modalprism");
    host.set_expose(ops_expose());
    let grant = AbilityGrant {
        app: "modalprism".into(),
        open: false,
    };

    let providers: Vec<(
        &str,
        Arc<dyn ability_support::AbilityProvider>,
        bool,
        Option<Value>,
    )> = vec![
        (
            "strata_read",
            ability_strata_read::provider(),
            true,
            Some(strata_config.clone()),
        ),
        (
            "strata_write",
            ability_strata_write::provider(),
            false,
            Some(strata_config.clone()),
        ),
        (
            "strata_queue",
            ability_strata_queue::provider(),
            false,
            Some(strata_config.clone()),
        ),
        (
            "control_state",
            ability_control_state::provider(),
            false,
            Some(strata_config.clone()),
        ),
        (
            "config",
            ability_config::provider(),
            false,
            Some(bind.clone()),
        ),
        (
            "config_inspect",
            ability_config_inspect::provider(),
            true,
            Some(bind.clone()),
        ),
        (
            "committer",
            ability_committer::provider(),
            false,
            Some(strata_config.clone()),
        ),
        (
            "egress",
            ability_egress::provider(),
            false,
            Some(bind.clone()),
        ),
        (
            "paid_gateway",
            ability_paid_gateway::provider(),
            false,
            Some(bind.clone()),
        ),
        (
            "planner",
            ability_collector_planner::provider(),
            true,
            Some(bind.clone()),
        ),
        (
            "runner",
            ability_collector_runner::provider(),
            false,
            Some(bind.clone()),
        ),
        (
            "health",
            ability_health::provider(),
            true,
            Some(bind.clone()),
        ),
        (
            "push_ingress",
            ability_push_ingress::provider(),
            false,
            Some(bind.clone()),
        ),
        (
            "accessor_http",
            ability_accessor_http::provider(),
            false,
            Some(bind.clone()),
        ),
        (
            "accessor_rss",
            ability_accessor_rss::provider(),
            false,
            Some(bind.clone()),
        ),
        (
            "accessor_api",
            ability_accessor_api::provider(),
            false,
            Some(bind.clone()),
        ),
        (
            "accessor_paid_api",
            ability_accessor_paid_api::provider(),
            false,
            Some(bind.clone()),
        ),
        (
            "accessor_search",
            ability_accessor_search::provider(),
            false,
            Some(bind.clone()),
        ),
        (
            "accessor_push",
            ability_accessor_push::provider(),
            false,
            Some(bind.clone()),
        ),
        (
            "accessor_file",
            ability_accessor_file::provider(),
            false,
            Some(bind.clone()),
        ),
        (
            "accessor_browser",
            ability_accessor_browser::provider(),
            false,
            Some(bind.clone()),
        ),
    ];

    for (id, provider, http, bind_config) in providers {
        host.set_ability_grant(id, grant.clone());
        host.install_str(id, provider)
            .await
            .map_err(|error| Error::new(error.to_string()))?;
        let kinds = if http {
            vec![AccessKind::Fun, AccessKind::Http]
        } else {
            vec![AccessKind::Fun]
        };
        host.set_protocol_support(id, kinds);
        let source = SourceId::new(id).map_err(|error| Error::new(error.to_string()))?;
        let instance = InstanceId::new(id).map_err(|error| Error::new(error.to_string()))?;
        let spec = LoadSpec::new(source, instance);
        let spec = match bind_config {
            Some(config) => spec.with_config(config),
            None => spec,
        };
        let spec = match id {
            "planner" => with_bindings(
                spec,
                &["config", "strata_queue", "control_state"],
            )?,
            "runner" => with_bindings(
                spec,
                &[
                    "config",
                    "strata_queue",
                    "strata_write",
                    "strata_read",
                    "egress",
                    "paid_gateway",
                    "control_state",
                    "accessor_http",
                    "accessor_rss",
                    "accessor_api",
                    "accessor_paid_api",
                    "accessor_search",
                    "accessor_push",
                    "accessor_file",
                    "accessor_browser",
                ],
            )?,
            "health" => with_bindings(spec, &["config", "planner", "strata_write"])?,
            "push_ingress" => with_bindings(
                spec,
                &["config", "egress", "paid_gateway", "strata_write"],
            )?,
            "committer" => with_bindings(spec, &["config"])?,
            _ => spec,
        };
        host.load(spec)
            .await
            .map_err(|error| Error::new(error.to_string()))?;
    }
    Ok(host)
}

fn with_bindings(spec: LoadSpec, deps: &[&str]) -> Result<LoadSpec, Error> {
    let mut bindings = BTreeMap::new();
    for dep in deps {
        bindings.insert(
            (*dep).to_string(),
            InstanceId::new(*dep).map_err(|error| Error::new(error.to_string()))?,
        );
    }
    Ok(LoadSpec {
        bindings: Some(bindings),
        ..spec
    })
}

fn spawn_planner_loop(host: Arc<AbilityHost>, stop: Arc<AtomicBool>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let Ok(value) = invoke_host(&host, "planner", "tick", json!({})).await else {
                continue;
            };
            let Some(tasks) = value.as_array() else {
                continue;
            };
            for task in tasks {
                let Ok(value) =
                    invoke_host(&host, "runner", "run_task", json!({ "task": task })).await
                else {
                    continue;
                };
                let Some(follows) = value.get("follows").and_then(Value::as_array) else {
                    continue;
                };
                if follows.is_empty() {
                    continue;
                }
                let _ = invoke_host(&host, "committer", "flush", json!({})).await;
                for follow in follows {
                    let _ = invoke_host(
                        &host,
                        "runner",
                        "run_task",
                        json!({ "task": follow }),
                    )
                    .await;
                }
            }
        }
    })
}

fn spawn_flush_loop(host: Arc<AbilityHost>, stop: Arc<AtomicBool>) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let applied = match invoke_host(&host, "config", "applied", json!({})).await {
                Ok(value) => value,
                Err(_) => {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };
            let wait = flush_interval_from_applied(&applied);
            tokio::time::sleep(wait).await;
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let Ok(value) = invoke_host(&host, "committer", "flush", json!({})).await else {
                continue;
            };
            if value.get("snapshot").is_some_and(|item| !item.is_null()) {
                let _ = apply_hotlist(&host).await;
            }
        }
    })
}

fn spawn_health_loop(host: Arc<AbilityHost>, stop: Arc<AtomicBool>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let _ = invoke_host(&host, "health", "evaluate", json!({})).await;
        }
    })
}

fn flush_interval_from_applied(applied: &Value) -> Duration {
    let generation = applied
        .get("generation")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    let system = applied.get("system").cloned().unwrap_or(json!({}));
    let modules = applied
        .get("modules")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let snapshot = config::Snapshot {
        generation,
        system,
        modules,
        origins: BTreeMap::new(),
    };
    collector_runtime::plans::flush_interval(&snapshot)
}

pub async fn apply_hotlist(host: &AbilityHost) -> Result<bool, types::Error> {
    let value = invoke_host(host, "committer", "hotlist_apply", json!({}))
        .await
        .map_err(|e| types::Error::new(e.to_string()))?;
    Ok(value
        .get("applied")
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

async fn serve_ops(addr: SocketAddr, gateway: Arc<dyn Gateway>) {
    use std::path::PathBuf;
    use axum::response::Redirect;
    use tower_http::services::{ServeDir, ServeFile};

    let state = console_api::ConsoleState {
        gateway: Arc::clone(&gateway),
        http_addr: addr.to_string(),
    };
    let console = if let Ok(dist) = std::env::var("MODALPRISM_CONSOLE_DIST") {
        let dist = PathBuf::from(dist);
        let index = dist.join("index.html");
        let service = ServeDir::new(dist)
            .append_index_html_on_directories(true)
            .not_found_service(ServeFile::new(index));
        axum::Router::new().fallback_service(service)
    } else {
        let dist = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../console-web/dist");
        if dist.join("index.html").is_file() && !console_static::has_embedded_assets() {
            let index = dist.join("index.html");
            let service = ServeDir::new(dist)
                .append_index_html_on_directories(true)
                .not_found_service(ServeFile::new(index));
            axum::Router::new().fallback_service(service)
        } else {
            axum::Router::new().fallback(console_static::serve)
        }
    };
    let app = axum::Router::new()
        .merge(console_api::router(state))
        .route(
            "/console/",
            axum::routing::get(|| async { Redirect::permanent("/console") }),
        )
        .nest("/console", console)
        .merge(HttpPort::router(gateway))
        .layer(axum::middleware::map_response(strip_cors));
    let Ok(listener) = TcpListener::bind(addr).await else {
        return;
    };
    let _ = axum::serve(listener, app).await;
}

async fn strip_cors(mut response: axum::response::Response) -> axum::response::Response {
    let names: Vec<_> = response
        .headers()
        .keys()
        .filter(|name| name.as_str().starts_with("access-control-"))
        .cloned()
        .collect();
    for name in names {
        response.headers_mut().remove(name);
    }
    let vary = response
        .headers()
        .get("vary")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    if let Some(vary) = vary {
        let kept: Vec<_> = vary
            .split(',')
            .map(str::trim)
            .filter(|token| !token.is_empty() && !cors_vary(token))
            .collect();
        if kept.is_empty() {
            response.headers_mut().remove("vary");
        } else if let Ok(value) = kept.join(", ").parse() {
            response.headers_mut().insert("vary", value);
        }
    }
    response
}

fn cors_vary(token: &str) -> bool {
    token.eq_ignore_ascii_case("origin")
        || token.eq_ignore_ascii_case("access-control-request-method")
        || token.eq_ignore_ascii_case("access-control-request-headers")
}

async fn invoke_host(
    host: &AbilityHost,
    instance: &str,
    method: &str,
    args: Value,
) -> Result<Value, ability_support::Error> {
    let instance = InstanceId::new(instance)
        .map_err(|error| ability_support::Error::invalid_args(error.to_string()))?;
    let method = MethodId::new(method)
        .map_err(|error| ability_support::Error::invalid_args(error.to_string()))?;
    let response = host
        .invoke(InvokeRequest::new(instance, method, args))
        .await?;
    Ok(response.data)
}

async fn invoke_gateway(
    gateway: &Arc<dyn Gateway>,
    instance: &str,
    method: &str,
    args: Value,
) -> Result<Value, String> {
    let instance = InstanceId::new(instance).map_err(|e| e.to_string())?;
    let method = MethodId::new(method).map_err(|e| e.to_string())?;
    let response = gateway
        .invoke(InvokeRequest::new(instance, method, args))
        .await
        .map_err(|e| e.to_string())?;
    Ok(response.data)
}
