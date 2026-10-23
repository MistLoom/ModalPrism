//! 采集进程。组合根直接链接 Ability，并持有调度、发布和健康循环。

pub mod ops;

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ability_core::AbilityHost;
use ability_support::{InstanceId, LoadSpec, MethodId, SourceId};
use assembly::{Binds, Services, policy_of};
use frame_core::{AccessKind, Gateway, InvokeRequest};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

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
    let gateway = ops::gateway(Arc::clone(&host));
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
    let http_gateway = Arc::clone(&host);
    tasks.push(tokio::spawn(async move {
        ops::serve(http, http_gateway).await;
    }));
    wait_http(http).await?;
    let (metadata, blobs) = services.strata().backend_names();
    let upstream = services
        .upstream_proxy()
        .map(|route| route.display())
        .unwrap_or_else(|| "direct".into());
    println!(
        "modalprism: data={} metadata={metadata} blobs={blobs} vectors={} http=http://{http} proxy=http://{proxy} push=http://{push} upstream={upstream} mode={}",
        services.data_dir().display(),
        if services.strata().vector_accelerated() {
            "sqlite-vec"
        } else {
            "scan"
        },
        services.proxy_setting().label(),
    );
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
    let read: Arc<dyn types::ReadCtx> = Arc::clone(&services) as Arc<dyn types::ReadCtx>;
    let providers: Vec<(
        &str,
        std::sync::Arc<dyn ability_support::AbilityProvider>,
        bool,
    )> = vec![
        (
            "strata_read",
            strata_read::provider(Arc::clone(&read)),
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
        ("health", health::provider(Arc::clone(&read)), true),
        (
            "health_check",
            health_check::provider(Arc::clone(&services)),
            false,
        ),
        (
            "config",
            ability_config::provider(Arc::clone(&services)),
            false,
        ),
        (
            "config_inspect",
            config_inspect::provider(Arc::clone(&read)),
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
            "accessor_probe",
            accessor_probe::provider(Arc::clone(&services)),
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
            let value = match invoke(&host_tick, "planner", "tick", json!({})).await {
                Ok(value) => value,
                Err(error) => {
                    tracing::warn!(error = %error, "调度 tick 失败");
                    continue;
                }
            };
            let Some(tasks) = value.as_array() else {
                continue;
            };
            tracing::debug!(tasks = tasks.len(), "调度派发");
            for task in tasks {
                let value =
                    match invoke(&host_tick, "runner", "run_task", json!({ "task": task })).await {
                        Ok(value) => value,
                        Err(error) => {
                            tracing::warn!(error = %error, "任务执行失败");
                            continue;
                        }
                    };
                let Some(follows) = value.get("follows").and_then(Value::as_array) else {
                    continue;
                };
                if follows.is_empty() {
                    continue;
                }
                let _ = invoke(&host_tick, "committer", "flush", json!({})).await;
                for follow in follows {
                    let _ =
                        invoke(&host_tick, "runner", "run_task", json!({ "task": follow })).await;
                }
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
            let value = match invoke(&host_flush, "committer", "flush", json!({})).await {
                Ok(value) => value,
                Err(error) => {
                    tracing::warn!(error = %error, "提交失败");
                    continue;
                }
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
            if let Err(error) = invoke(&host, "health_check", "evaluate", json!({})).await {
                tracing::warn!(error = %error, "健康评估失败");
            }
        }
    });
    vec![tick, flush, health]
}

pub async fn apply_hotlist(services: &Services) -> Result<bool, types::Error> {
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
