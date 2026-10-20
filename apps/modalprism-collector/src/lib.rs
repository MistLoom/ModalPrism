//! 采集进程。组合根直接链接 Ability，并持有调度、发布和健康循环。

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ability_core::AbilityHost;
use ability_support::{InstanceId, LoadSpec, MethodId, SourceId};
use assembly::{Binds, Services, policy_of};
use async_trait::async_trait;
use frame_core::{AccessKind, CatalogEntry, Gateway, InvokeRequest, InvokeResponse};
use frame_port_http::HttpPort;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

const OPS: &[&str] = &["strata_read", "planner", "health"];

pub struct Running {
    services: Arc<Services>,
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
    pub fn services(&self) -> &Services {
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

    pub async fn shutdown(self) -> Result<(), assembly::Error> {
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
) -> Result<Running, assembly::Error> {
    let http_addr = binds.http;
    let proxy_addr = binds.proxy;
    let push_addr = binds.push;
    let services = Services::open(data, config_dir, binds).await?;
    let proxy_listener = TcpListener::bind(proxy_addr)
        .await
        .map_err(|source| assembly::Error::with_source("监听", source))?;
    let proxy = proxy_listener
        .local_addr()
        .map_err(|source| assembly::Error::with_source("监听", source))?;
    services.set_proxy(format!("http://{proxy}"));
    let push_listener = TcpListener::bind(push_addr)
        .await
        .map_err(|source| assembly::Error::with_source("监听", source))?;
    let push = push_listener
        .local_addr()
        .map_err(|source| assembly::Error::with_source("监听", source))?;
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
        assembly::serve_proxy(proxy_gateway, proxy_listener, move || {
            policy_of(&proxy_services.config().applied().system)
        })
        .await;
    }));
    let push_services = Arc::clone(&services);
    tasks.push(tokio::spawn(async move {
        assembly::serve_push(push_services, push_listener).await;
    }));
    let http_gateway = Arc::clone(&gateway);
    tasks.push(tokio::spawn(async move {
        serve_ops(http, http_gateway).await;
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

async fn reserved_http(addr: SocketAddr) -> Result<SocketAddr, assembly::Error> {
    if addr.port() != 0 {
        return Ok(addr);
    }
    let probe = TcpListener::bind(addr)
        .await
        .map_err(|source| assembly::Error::with_source("监听", source))?;
    let local = probe
        .local_addr()
        .map_err(|source| assembly::Error::with_source("监听", source))?;
    drop(probe);
    Ok(local)
}

async fn wait_http(addr: SocketAddr) -> Result<(), assembly::Error> {
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Err(assembly::Error::with_source(
        "监听",
        std::io::Error::other("监听"),
    ))
}

async fn link(services: Arc<Services>) -> Result<Arc<AbilityHost>, assembly::Error> {
    let host = Arc::new(AbilityHost::new());
    host.set_entry_app("modalprism");
    let providers: Vec<(
        &str,
        std::sync::Arc<dyn ability_support::AbilityProvider>,
        bool,
    )> = vec![
        (
            "strata_read",
            strata_read::provider(Arc::clone(&services)),
            true,
        ),
        (
            "strata_write",
            strata_write::provider(Arc::clone(&services)),
            false,
        ),
        (
            "strata_queue",
            strata_queue::provider(Arc::clone(&services)),
            false,
        ),
        (
            "committer",
            committer::provider(Arc::clone(&services)),
            false,
        ),
        (
            "planner",
            ability_planner::provider(Arc::clone(&services)),
            true,
        ),
        ("runner", runner::provider(Arc::clone(&services)), false),
        ("health", health::provider(Arc::clone(&services)), true),
        (
            "config",
            ability_config::provider(Arc::clone(&services)),
            false,
        ),
        (
            "config_inspect",
            config_inspect::provider(Arc::clone(&services)),
            false,
        ),
        (
            "egress",
            ability_egress::provider(Arc::clone(&services)),
            false,
        ),
        (
            "paid_gateway",
            paid_gateway::provider(Arc::clone(&services)),
            false,
        ),
        (
            "accessor_http",
            accessor_http::provider(Arc::clone(&services)),
            false,
        ),
        (
            "accessor_rss",
            accessor_rss::provider(Arc::clone(&services)),
            false,
        ),
        (
            "accessor_api",
            accessor_api::provider(Arc::clone(&services)),
            false,
        ),
        (
            "accessor_paid_api",
            accessor_paid_api::provider(Arc::clone(&services)),
            false,
        ),
        (
            "accessor_search",
            accessor_search::provider(Arc::clone(&services)),
            false,
        ),
        (
            "accessor_push",
            accessor_push::provider(Arc::clone(&services)),
            false,
        ),
        (
            "accessor_file",
            accessor_file::provider(Arc::clone(&services)),
            false,
        ),
        (
            "accessor_browser",
            accessor_browser::provider(Arc::clone(&services)),
            false,
        ),
    ];
    for (id, provider, http) in providers {
        host.install_str(id, provider)
            .await
            .map_err(|error| assembly::Error::new(error.to_string()))?;
        let kinds = if http {
            vec![AccessKind::Fun, AccessKind::Http]
        } else {
            vec![AccessKind::Fun]
        };
        host.set_protocol_support(id, kinds);
        let source = SourceId::new(id).map_err(|error| assembly::Error::new(error.to_string()))?;
        let instance =
            InstanceId::new(id).map_err(|error| assembly::Error::new(error.to_string()))?;
        host.load(LoadSpec::new(source, instance))
            .await
            .map_err(|error| assembly::Error::new(error.to_string()))?;
    }
    Ok(host)
}

fn spawn_loops(
    host: Arc<AbilityHost>,
    services: Arc<Services>,
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
                let _ = invoke(&host_tick, "runner", "run_task", json!({ "task": task })).await;
            }
        }
    });
    let host_flush = Arc::clone(&host);
    let services_flush = Arc::clone(&services);
    let stop_flush = Arc::clone(&stop);
    let flush = tokio::spawn(async move {
        loop {
            let wait = assembly::flush_interval(&services_flush.config().applied());
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

pub async fn apply_hotlist(services: &Services) -> Result<bool, types::Error> {
    let applied = services.config().applied();
    let Some(changeset) =
        types::HotList::changeset(services.strata().as_ref(), applied.as_ref()).await?
    else {
        return Ok(false);
    };
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

async fn serve_ops(addr: SocketAddr, gateway: Arc<dyn Gateway>) {
    let app = HttpPort::router(gateway).layer(axum::middleware::map_response(strip_cors));
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
