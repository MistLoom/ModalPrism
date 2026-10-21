//! 采集进程。组合根直接链接 Ability，并持有调度、发布和健康循环。

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ability_core::{AbilityGrant, AbilityHost};
use ability_support::{InstanceId, LoadSpec, MethodId, SourceId};
use collector_boot::{BootResources, flush_interval, serve_egress_proxy};
use collector_gateway::policy_of;
use collector_runtime::{Binds, Error, serve_push};
use frame_core::{AbilityExpose, AccessKind, Gateway, InvokeRequest};
use frame_port_http::HttpPort;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

mod console_static;
mod providers;

pub struct Running {
    services: Arc<BootResources>,
    host: Arc<AbilityHost>,
    gateway: Arc<dyn Gateway>,
    http: SocketAddr,
    proxy: SocketAddr,
    push: SocketAddr,
    stop: Arc<AtomicBool>,
    tasks: Vec<JoinHandle<()>>,
    _watch: config::WatchHandle,
}

impl Running {
    pub fn services(&self) -> &BootResources {
        &self.services
    }

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
        self.services.shutdown().await
    }
}

pub async fn boot(
    data: &Path,
    config_dir: &Path,
    binds: Binds,
) -> Result<Running, Error> {
    let http_addr = binds.http;
    let proxy_addr = binds.proxy;
    let push_addr = binds.push;
    let services = BootResources::open(data, config_dir, binds).await?;
    let proxy_listener = TcpListener::bind(proxy_addr)
        .await
        .map_err(|source| Error::with_source("监听", source))?;
    let proxy = proxy_listener
        .local_addr()
        .map_err(|source| Error::with_source("监听", source))?;
    services.set_proxy(format!("http://{proxy}"));
    let push_listener = TcpListener::bind(push_addr)
        .await
        .map_err(|source| Error::with_source("监听", source))?;
    let push = push_listener
        .local_addr()
        .map_err(|source| Error::with_source("监听", source))?;
    let http = reserved_http(http_addr).await?;
    let host = link(Arc::clone(&services)).await?;
    host.set_access_gate(Some(AccessKind::Http));
    let gateway: Arc<dyn Gateway> = host.as_ref().clone().into_gateway();
    let stop = Arc::new(AtomicBool::new(false));
    let mut tasks = Vec::new();
    let proxy_gateway = services.egress_gateway();
    let proxy_services = Arc::clone(&services);
    tasks.push(tokio::spawn(async move {
        serve_egress_proxy(proxy_gateway, proxy_listener, move || {
            policy_of(&proxy_services.config().applied().system)
        })
        .await;
    }));
    let push_services = Arc::clone(&services);
    tasks.push(tokio::spawn(async move {
        serve_push(push_services, push_listener).await;
    }));
    let http_gateway = Arc::clone(&gateway);
    let http_services = Arc::clone(&services);
    tasks.push(tokio::spawn(async move {
        serve_ops(http, http_gateway, http_services).await;
    }));
    wait_http(http).await?;
    tasks.extend(spawn_loops(
        Arc::clone(&host),
        Arc::clone(&services),
        Arc::clone(&stop),
    ));
    let watch = config::watch(services.config(), Duration::from_secs(2))?;
    Ok(Running {
        services,
        host,
        gateway,
        http,
        proxy,
        push,
        stop,
        tasks,
        _watch: watch,
    })
}

async fn reserved_http(addr: SocketAddr) -> Result<SocketAddr, Error> {
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

fn bind_config(services: &BootResources) -> Value {
    json!({
        "dataDir": services.data_dir(),
        "configDir": services.config_dir(),
        "system": services.config().applied().system,
        "proxyUrl": services.proxy_url(),
        "browserProgram": services.browser_program(),
    })
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

async fn link(services: Arc<BootResources>) -> Result<Arc<AbilityHost>, Error> {
    let host = Arc::new(AbilityHost::new());
    host.set_entry_app("modalprism");
    host.set_expose(ops_expose());
    let bind = bind_config(&services);
    let strata_config = json!({
        "dataDir": services.data_dir(),
        "system": services.config().applied().system,
    });
    let grant = AbilityGrant {
        app: "modalprism".into(),
        open: false,
    };
    let providers: Vec<(
        &str,
        std::sync::Arc<dyn ability_support::AbilityProvider>,
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
            "committer",
            ability_committer::provider(),
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
            providers::health::provider(Arc::clone(&services)),
            true,
            None,
        ),
        (
            "accessor_http",
            providers::accessor::provider(Arc::clone(&services), "accessor_http", "http"),
            false,
            None,
        ),
        (
            "accessor_rss",
            providers::accessor::provider(Arc::clone(&services), "accessor_rss", "rss"),
            false,
            None,
        ),
        (
            "accessor_api",
            providers::accessor::provider(Arc::clone(&services), "accessor_api", "api"),
            false,
            None,
        ),
        (
            "accessor_paid_api",
            providers::accessor::provider(Arc::clone(&services), "accessor_paid_api", "paid-api"),
            false,
            None,
        ),
        (
            "accessor_search",
            providers::accessor::provider(Arc::clone(&services), "accessor_search", "search"),
            false,
            None,
        ),
        (
            "accessor_push",
            providers::accessor::provider(Arc::clone(&services), "accessor_push", "push"),
            false,
            None,
        ),
        (
            "accessor_file",
            providers::accessor::provider(Arc::clone(&services), "accessor_file", "file"),
            false,
            None,
        ),
        (
            "accessor_browser",
            providers::accessor::provider(Arc::clone(&services), "accessor_browser", "browser"),
            false,
            None,
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
        let instance =
            InstanceId::new(id).map_err(|error| Error::new(error.to_string()))?;
        let spec = LoadSpec::new(source, instance);
        let spec = match bind_config {
            Some(config) => spec.with_config(config),
            None => spec,
        };
        let spec = match id {
            "planner" => {
                let mut bindings = BTreeMap::new();
                for dep in ["config", "strata_queue", "control_state"] {
                    bindings.insert(
                        dep.to_string(),
                        InstanceId::new(dep).map_err(|error| Error::new(error.to_string()))?,
                    );
                }
                LoadSpec {
                    bindings: Some(bindings),
                    ..spec
                }
            }
            "runner" => {
                let mut bindings = BTreeMap::new();
                for dep in [
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
                ] {
                    bindings.insert(
                        dep.to_string(),
                        InstanceId::new(dep).map_err(|error| Error::new(error.to_string()))?,
                    );
                }
                LoadSpec {
                    bindings: Some(bindings),
                    ..spec
                }
            }
            _ => spec,
        };
        host.load(spec)
            .await
            .map_err(|error| Error::new(error.to_string()))?;
    }
    Ok(host)
}

fn spawn_loops(
    host: Arc<AbilityHost>,
    services: Arc<BootResources>,
    stop: Arc<AtomicBool>,
) -> Vec<JoinHandle<()>> {
    let host_tick = Arc::clone(&host);
    let stop_tick = Arc::clone(&stop);
    let tick = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            if stop_tick.load(Ordering::Relaxed) {
                break;
            }
            let Ok(value) = invoke(&host_tick, "planner", "tick", json!({})).await else {
                continue;
            };
            let Some(tasks) = value.as_array() else {
                continue;
            };
            for task in tasks {
                let Ok(value) = invoke(&host_tick, "runner", "run_task", json!({ "task": task })).await
                else {
                    continue;
                };
                let Some(follows) = value.get("follows").and_then(Value::as_array) else {
                    continue;
                };
                if follows.is_empty() {
                    continue;
                }
                let _ = invoke(&host_tick, "committer", "flush", json!({})).await;
                for follow in follows {
                    let _ = invoke(
                        &host_tick,
                        "runner",
                        "run_task",
                        json!({ "task": follow }),
                    )
                    .await;
                }
            }
        }
    });
    let host_flush = Arc::clone(&host);
    let services_flush = Arc::clone(&services);
    let stop_flush = Arc::clone(&stop);
    let flush = tokio::spawn(async move {
        loop {
            let wait = flush_interval(&services_flush.config().applied());
            tokio::time::sleep(wait).await;
            if stop_flush.load(Ordering::Relaxed) {
                break;
            }
            let Ok(value) = invoke(&host_flush, "committer", "flush", json!({})).await else {
                continue;
            };
            if value.get("snapshot").is_some_and(|item| !item.is_null()) {
                let _ = apply_hotlist(&services_flush).await;
            }
        }
    });
    let stop_health = stop;
    let health = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            if stop_health.load(Ordering::Relaxed) {
                break;
            }
            let _ = invoke(&host, "health", "evaluate", json!({})).await;
        }
    });
    vec![tick, flush, health]
}

pub async fn apply_hotlist(services: &BootResources) -> Result<bool, types::Error> {
    let applied = services.config().applied();
    let value = types::HotList::on_commit(services.strata().as_ref(), applied.as_ref()).await?;
    if value.is_null() {
        return Ok(false);
    }
    let changeset = serde_json::from_value(value)
        .map_err(|source| types::Error::with_source(source.to_string(), source))?;
    services
        .strata()
        .apply(changeset, strata_model::ApplyOpts::default())
        .await
        .map_err(|error| types::Error::new(error.to_string()))?;
    services
        .strata()
        .flush()
        .await
        .map_err(|error| types::Error::new(error.to_string()))?;
    Ok(true)
}

async fn serve_ops(addr: SocketAddr, gateway: Arc<dyn Gateway>, services: Arc<BootResources>) {
    use std::path::PathBuf;
    use axum::response::Redirect;
    use tower_http::services::{ServeDir, ServeFile};

    let state = console_api::ConsoleState {
        gateway: Arc::clone(&gateway),
        services,
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

async fn invoke(
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
