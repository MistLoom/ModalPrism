//! 采集进程。组合根直接链接 Ability，并持有调度、发布和健康循环。

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ability_core::AbilityHost;
use ability_support::{InstanceId, LoadSpec, MethodId, SourceId};
use host::{Binds, HostResources, policy_of};
use async_trait::async_trait;
use frame_core::{AccessKind, CatalogEntry, Gateway, InvokeRequest, InvokeResponse};
use frame_port_http::HttpPort;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

mod console_static;

const OPS: &[&str] = &["strata_read", "planner", "health"];

pub struct Running {
    services: Arc<HostResources>,
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
    pub fn services(&self) -> &HostResources {
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

    pub async fn shutdown(self) -> Result<(), host::Error> {
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
) -> Result<Running, host::Error> {
    let http_addr = binds.http;
    let proxy_addr = binds.proxy;
    let push_addr = binds.push;
    let services = HostResources::open(data, config_dir, binds).await?;
    let proxy_listener = TcpListener::bind(proxy_addr)
        .await
        .map_err(|source| host::Error::with_source("监听", source))?;
    let proxy = proxy_listener
        .local_addr()
        .map_err(|source| host::Error::with_source("监听", source))?;
    services.set_proxy(format!("http://{proxy}"));
    let push_listener = TcpListener::bind(push_addr)
        .await
        .map_err(|source| host::Error::with_source("监听", source))?;
    let push = push_listener
        .local_addr()
        .map_err(|source| host::Error::with_source("监听", source))?;
    let http = reserved_http(http_addr).await?;
    let host = link(Arc::clone(&services)).await?;
    let gateway: Arc<dyn Gateway> = Arc::new(OpsGateway {
        inner: Arc::clone(&host),
    });
    let stop = Arc::new(AtomicBool::new(false));
    let mut tasks = Vec::new();
    let proxy_gateway = services.egress_gateway();
    let proxy_services = Arc::clone(&services);
    tasks.push(tokio::spawn(async move {
        host::serve_proxy(proxy_gateway, proxy_listener, move || {
            policy_of(&proxy_services.config().applied().system)
        })
        .await;
    }));
    let push_services = Arc::clone(&services);
    tasks.push(tokio::spawn(async move {
        host::serve_push(push_services, push_listener).await;
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

async fn reserved_http(addr: SocketAddr) -> Result<SocketAddr, host::Error> {
    if addr.port() != 0 {
        return Ok(addr);
    }
    let probe = TcpListener::bind(addr)
        .await
        .map_err(|source| host::Error::with_source("监听", source))?;
    let local = probe
        .local_addr()
        .map_err(|source| host::Error::with_source("监听", source))?;
    drop(probe);
    Ok(local)
}

async fn wait_http(addr: SocketAddr) -> Result<(), host::Error> {
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Err(host::Error::with_source(
        "监听",
        std::io::Error::other("监听"),
    ))
}

fn strata_bind_config(services: &HostResources) -> Value {
    json!({
        "dataDir": services.data_dir(),
        "system": services.config().applied().system,
    })
}

async fn link(services: Arc<HostResources>) -> Result<Arc<AbilityHost>, host::Error> {
    let host = Arc::new(AbilityHost::new());
    host.set_entry_app("modalprism");
    let strata_config = strata_bind_config(&services);
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
            "committer",
            committer::provider(Arc::clone(&services)),
            false,
            None,
        ),
        (
            "planner",
            ability_planner::provider(Arc::clone(&services)),
            true,
            None,
        ),
        ("runner", runner::provider(Arc::clone(&services)), false, None),
        ("health", health::provider(Arc::clone(&services)), true, None),
        (
            "config",
            ability_config::provider(Arc::clone(&services)),
            false,
            None,
        ),
        (
            "config_inspect",
            config_inspect::provider(Arc::clone(&services)),
            false,
            None,
        ),
        (
            "egress",
            ability_egress::provider(Arc::clone(&services)),
            false,
            None,
        ),
        (
            "paid_gateway",
            paid_gateway::provider(Arc::clone(&services)),
            false,
            None,
        ),
        (
            "accessor_http",
            accessor_http::provider(Arc::clone(&services)),
            false,
            None,
        ),
        (
            "accessor_rss",
            accessor_rss::provider(Arc::clone(&services)),
            false,
            None,
        ),
        (
            "accessor_api",
            accessor_api::provider(Arc::clone(&services)),
            false,
            None,
        ),
        (
            "accessor_paid_api",
            accessor_paid_api::provider(Arc::clone(&services)),
            false,
            None,
        ),
        (
            "accessor_search",
            accessor_search::provider(Arc::clone(&services)),
            false,
            None,
        ),
        (
            "accessor_push",
            accessor_push::provider(Arc::clone(&services)),
            false,
            None,
        ),
        (
            "accessor_file",
            accessor_file::provider(Arc::clone(&services)),
            false,
            None,
        ),
        (
            "accessor_browser",
            accessor_browser::provider(Arc::clone(&services)),
            false,
            None,
        ),
    ];
    for (id, provider, http, bind_config) in providers {
        host.install_str(id, provider)
            .await
            .map_err(|error| host::Error::new(error.to_string()))?;
        let kinds = if http {
            vec![AccessKind::Fun, AccessKind::Http]
        } else {
            vec![AccessKind::Fun]
        };
        host.set_protocol_support(id, kinds);
        let source = SourceId::new(id).map_err(|error| host::Error::new(error.to_string()))?;
        let instance =
            InstanceId::new(id).map_err(|error| host::Error::new(error.to_string()))?;
        let spec = LoadSpec::new(source, instance);
        let spec = match bind_config {
            Some(config) => spec.with_config(config),
            None => spec,
        };
        host.load(spec)
            .await
            .map_err(|error| host::Error::new(error.to_string()))?;
    }
    Ok(host)
}

fn spawn_loops(
    host: Arc<AbilityHost>,
    services: Arc<HostResources>,
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
            let wait = host::flush_interval(&services_flush.config().applied());
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

pub async fn apply_hotlist(services: &HostResources) -> Result<bool, types::Error> {
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

async fn serve_ops(addr: SocketAddr, gateway: Arc<dyn Gateway>, services: Arc<HostResources>) {
    use std::path::PathBuf;
    use axum::response::Redirect;
    use tower_http::services::{ServeDir, ServeFile};

    let state = console_api::ConsoleState {
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

struct OpsGateway {
    inner: Arc<AbilityHost>,
}

#[async_trait]
impl Gateway for OpsGateway {
    async fn invoke(&self, request: InvokeRequest) -> ability_support::Result<InvokeResponse> {
        if !OPS.contains(&request.instance.as_str()) {
            return Err(ability_support::Error::unavailable("实例未开放"));
        }
        self.inner.invoke(request).await
    }

    fn catalog(&self) -> Vec<CatalogEntry> {
        self.inner
            .catalog()
            .into_iter()
            .filter(|entry| OPS.contains(&entry.instance.as_str()))
            .collect()
    }

    fn get_entry(&self, instance: &InstanceId, method: &MethodId) -> Option<CatalogEntry> {
        if !OPS.contains(&instance.as_str()) {
            return None;
        }
        self.inner.get_entry(instance, method)
    }
}
