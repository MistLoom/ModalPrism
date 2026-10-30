//! 采集进程。组合根直接链接 Ability，并持有调度、发布和健康循环。

pub mod ops;

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ability_core::AbilityHost;
use ability_support::{
    InstanceId, LoadSpec, MethodId, SourceId, discover_ability_roots, load_ability_decl,
};
use assembly::{Binds, Services, policy_of};
use frame_core::{AccessKind, Gateway, InvokeRequest};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

pub struct Running {
    services: Arc<Services>,
    host: Arc<AbilityHost>,
    gateway: Arc<dyn Gateway>,
    monitor: Arc<leaf_monitor::MonitorApp>,
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
        self.monitor.shutdown();
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
    boot_opts(data, config_dir, binds, BootOpts::default()).await
}

/// boot 可选项（测试接缝）。
#[derive(Clone, Debug, Default)]
pub struct BootOpts {
    /// 强制不安装 rawprism Guest：/parse 落回 412 前置失败（负向场景用）。
    pub disable_parse_guest: bool,
}

pub async fn boot_opts(
    data: &Path,
    config_dir: &Path,
    binds: Binds,
    opts: BootOpts,
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
    let http_listener = TcpListener::bind(http_addr)
        .await
        .map_err(|source| assembly::Error::with_source("监听", source))?;
    let http = http_listener
        .local_addr()
        .map_err(|source| assembly::Error::with_source("监听", source))?;
    let (host, parse_ready) = link(Arc::clone(&services), &opts).await?;
    // 框架监测：只在上层接上 monitor 并把内存视图注入领域 API，不改框架组件。
    let monitor_config = leaf_monitor::MonitorConfig {
        log_path: data.join("monitor.jsonl").display().to_string(),
        ..leaf_monitor::MonitorConfig::default()
    };
    let monitor = leaf_monitor::attach(&host, monitor_config);
    let gateway = ops::gateway(Arc::clone(&host));
    let api = domain_api::AppState::open(Arc::clone(&services), config_dir, data, http)
        .await
        .map_err(|error| assembly::Error::new(error.to_string()))?;
    // 接线结构化解析：rawprism 经仓颉 Guest 由 host 提供（Guest 未就绪时 /parse 返回 412）。
    // rawprism 是内部解析引擎而非治理面能力（仓颉 Guest、fun 协议、无 logic 面），
    // 所以钩子直连 host 而不走 OpsGateway 的 COLLECTOR_OPS 白名单。
    {
        let host = Arc::clone(&host);
        let hook: domain_api::ParseHook = Arc::new(move |args: &serde_json::Value| {
            let host = Arc::clone(&host);
            let args = args.clone();
            Box::pin(async move {
                let response = host
                    .invoke(frame_core::InvokeRequest::new(
                        InstanceId::new("rawprism").map_err(|e| e.to_string())?,
                        MethodId::new("scan").map_err(|e| e.to_string())?,
                        args,
                    ))
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(response.data)
            }) as domain_api::ParseHookFuture
        });
        // AppState 已被后台任务共享，钩子经 OnceLock 单写；写入失败只在真正冲突时告警。
        if parse_ready && api.parse_hook.set(hook).is_err() {
            tracing::warn!("parse 钩子已写入，忽略重复接线");
        }
    }
    // 接线监测投影：控制台工作台轮询 /monitor/summary，读的是同一份内存环形缓冲。
    {
        let monitor = Arc::clone(&monitor);
        let hook: domain_api::MonitorHook = Arc::new(move |what: &str, tail: usize| {
            match what {
                "chains" => json!({"available": true, "chains": monitor.chains()}),
                "events" => {
                    let events = monitor.recent_events();
                    let start = events.len().saturating_sub(tail.max(1));
                    json!({"available": true, "events": &events[start..]})
                }
                "stats" => {
                    let mut value = serde_json::to_value(monitor.stats()).unwrap_or_else(|_| json!({}));
                    value["available"] = json!(true);
                    value
                }
                "graph" => json!({"available": true, "graph": monitor.chain_graph()}),
                "summary" => {
                    let events = monitor.recent_events();
                    let start = events.len().saturating_sub(tail.max(1));
                    json!({
                        "available": true,
                        "stats": monitor.stats(),
                        "chains": monitor.chains(),
                        "events": &events[start..],
                    })
                }
                _ => json!({"available": false}),
            }
        });
        if api.monitor_hook.set(hook).is_err() {
            tracing::warn!("monitor 钩子已写入，忽略重复接线");
        }
    }
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
    let maintenance = Arc::clone(&api.maintenance);
    tasks.push(tokio::spawn(async move {
        if let Err(error) = axum::serve(http_listener, domain_api::router(api)).await {
            tracing::error!(%error, "领域 API 退出");
        }
    }));
    wait_http(http).await?;
    let (metadata, blobs) = services.strata().backend_names();
    let upstream = services
        .upstream_proxy()
        .map(|route| route.display())
        .unwrap_or_else(|| "direct".into());
    println!(
        "modalprism: data={} metadata={metadata} blobs={blobs} vectors={} http=http://{http} proxy=http://{proxy} push=http://{push} upstream={upstream} parse={} mode={}",
        services.data_dir().display(),
        if services.strata().vector_accelerated() {
            "sqlite-vec"
        } else {
            "scan"
        },
        services.proxy_setting().label(),
        if parse_ready { "ready" } else { "absent" },
    );
    tasks.extend(spawn_loops(
        Arc::clone(&host),
        Arc::clone(&services),
        Arc::clone(&stop),
        maintenance,
    ));
    let watch = config::watch(services.config(), Duration::from_secs(2))?;
    Ok(Running {
        services,
        host,
        gateway,
        monitor,
        http,
        proxy,
        push,
        stop,
        tasks,
        _watch: watch,
    })
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

fn protocol_kinds_for(workspace: &Path, id: &str) -> Vec<AccessKind> {
    for root in discover_ability_roots(workspace) {
        let yml = root.join("ability.yml");
        if let Ok(decl) = load_ability_decl(&yml) {
            if decl.id == id {
                return decl
                    .protocols
                    .iter()
                    .filter_map(|kind| AccessKind::from_protocol(*kind))
                    .collect();
            }
        }
    }
    Vec::new()
}
async fn link(
    services: Arc<Services>,
    opts: &BootOpts,
) -> Result<(Arc<AbilityHost>, bool), assembly::Error> {
    let host = Arc::new(AbilityHost::new());
    host.set_entry_app("modalprism");
    let read: Arc<dyn types::ReadCtx> = Arc::clone(&services) as Arc<dyn types::ReadCtx>;
    let providers: Vec<(&str, std::sync::Arc<dyn ability_support::AbilityProvider>)> = vec![
        ("strata_read", strata_read::provider(Arc::clone(&read))),
        (
            "strata_write",
            strata_write::provider(Arc::clone(&services)),
        ),
        (
            "strata_queue",
            strata_queue::provider(Arc::clone(&services)),
        ),
        (
            "content_admin",
            content_admin::provider(Arc::clone(&services)),
        ),
        ("committer", committer::provider(Arc::clone(&services))),
        ("planner", ability_planner::provider(Arc::clone(&services))),
        ("runner", runner::provider(Arc::clone(&services))),
        ("health", health::provider(Arc::clone(&read))),
        (
            "health_record",
            health_record::provider(Arc::clone(&services)),
        ),
        (
            "config_admin",
            ability_config_admin::provider(Arc::clone(&services)),
        ),
        (
            "config_inspect",
            config_inspect::provider(Arc::clone(&read)),
        ),
        ("egress", ability_egress::provider(Arc::clone(&services))),
        (
            "paid_gateway",
            paid_gateway::provider(Arc::clone(&services)),
        ),
        (
            "accessor_probe",
            accessor_probe::provider(Arc::clone(&services)),
        ),
        ("catalog_admin", catalog_admin::provider(Arc::clone(&read))),
        ("type_article", type_article::provider(Arc::clone(&read))),
    ];
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (id, provider) in providers {
        // 协议投影改从 ability.yml 读取（AbilityManifest 不再承载 protocols）。
        let kinds = protocol_kinds_for(&workspace, id);
        host.install_str(id, provider)
            .await
            .map_err(|error| assembly::Error::new(error.to_string()))?;
        host.set_protocol_support(id, kinds);
        let source = SourceId::new(id).map_err(|error| assembly::Error::new(error.to_string()))?;
        let instance =
            InstanceId::new(id).map_err(|error| assembly::Error::new(error.to_string()))?;
        host.load(LoadSpec::new(source, instance))
            .await
            .map_err(|error| assembly::Error::new(error.to_string()))?;
    }
    let parse_ready = install_parse_guest(&host, &workspace, opts).await;
    Ok((host, parse_ready))
}

/// 安装并加载 rawprism 仓颉 Guest（结构化解析）。失败不炸启动：
/// `/parse` 落回 412 前置失败，由 `parse=absent` 启动日志点明。
async fn install_parse_guest(host: &Arc<AbilityHost>, workspace: &Path, opts: &BootOpts) -> bool {
    if opts.disable_parse_guest {
        return false;
    }
    let provider =
        ability_core::GuestSourceProvider::from_workspace_rel(workspace, "abilitys/data/rawprism")
            .into_provider();
    let kinds = protocol_kinds_for(workspace, "rawprism");
    let source = match SourceId::new("rawprism") {
        Ok(source) => source,
        Err(error) => {
            tracing::warn!(%error, "rawprism Guest 源标识无效");
            return false;
        }
    };
    let instance = match InstanceId::new("rawprism") {
        Ok(instance) => instance,
        Err(error) => {
            tracing::warn!(%error, "rawprism Guest 实例标识无效");
            return false;
        }
    };
    if let Err(error) = host.install_str("rawprism", provider).await {
        tracing::warn!(%error, "rawprism Guest 安装失败（需 cjpm 工具链或预构建产物）");
        return false;
    }
    host.set_protocol_support("rawprism", kinds);
    if let Err(error) = host.load(LoadSpec::new(source, instance)).await {
        tracing::warn!(%error, "rawprism Guest 加载失败");
        return false;
    }
    true
}

fn spawn_loops(
    host: Arc<AbilityHost>,
    services: Arc<Services>,
    stop: Arc<AtomicBool>,
    maintenance: Arc<AtomicBool>,
) -> Vec<JoinHandle<()>> {
    let host_tick = Arc::clone(&host);
    let stop_tick = Arc::clone(&stop);
    let maintenance_tick = Arc::clone(&maintenance);
    let tick = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            if stop_tick.load(Ordering::Relaxed) {
                break;
            }
            if maintenance_tick.load(Ordering::Relaxed) {
                continue;
            }
            let value = match invoke(&host_tick, "planner", "round", json!({})).await {
                Ok(value) => value,
                Err(error) => {
                    tracing::warn!(error = %error, "调度派发失败");
                    continue;
                }
            };
            let leased = value.get("leased").and_then(Value::as_u64).unwrap_or(0);
            let executed = value
                .get("executed")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            let failed = value
                .get("failed")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            if leased > 0 {
                tracing::debug!(leased, executed, failed, "调度派发");
            }
        }
    });
    let host_sweep = Arc::clone(&host);
    let stop_sweep = Arc::clone(&stop);
    let maintenance_sweep = Arc::clone(&maintenance);
    let sweep = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await;
            if stop_sweep.load(Ordering::Relaxed) {
                break;
            }
            if maintenance_sweep.load(Ordering::Relaxed) {
                continue;
            }
            match invoke(&host_sweep, "committer", "sweep", json!({"dryRun": false})).await {
                Ok(value) => {
                    let tasks = value.get("tasks").and_then(Value::as_u64).unwrap_or(0);
                    let snapshots = value.get("snapshots").and_then(Value::as_u64).unwrap_or(0);
                    tracing::info!(tasks, snapshots, "保留清理完成");
                }
                // 清理失败只告警：保留策略不参与主链路成败。
                Err(error) => tracing::warn!(error = %error, "保留清理失败"),
            }
        }
    });
    let host_flush = Arc::clone(&host);
    let services_flush = Arc::clone(&services);
    let stop_flush = Arc::clone(&stop);
    let maintenance_flush = Arc::clone(&maintenance);
    let flush = tokio::spawn(async move {
        loop {
            let wait = assembly::flush_interval(&services_flush.config().applied());
            tokio::time::sleep(wait).await;
            if stop_flush.load(Ordering::Relaxed) {
                break;
            }
            if maintenance_flush.load(Ordering::Relaxed) {
                continue;
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
    let services_derive = Arc::clone(&services);
    let stop_derive = Arc::clone(&stop_health);
    let maintenance_derive = Arc::clone(&maintenance);
    let derive = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            if stop_derive.load(Ordering::Relaxed) {
                break;
            }
            if maintenance_derive.load(Ordering::Relaxed) {
                continue;
            }
            // 声明式派生的执行端：没有声明时是空操作。
            let run = match services_derive.run_derivations().await {
                Ok(run) => run,
                Err(error) => {
                    tracing::warn!(error = %error, "派生执行失败");
                    continue;
                }
            };
            if run.leased > 0 {
                tracing::info!(
                    leased = run.leased,
                    produced = run.produced,
                    skipped = run.skipped,
                    failed = run.failed,
                    "派生执行完成"
                );
            }
        }
    });
    let maintenance_health = Arc::clone(&maintenance);
    let health = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            if stop_health.load(Ordering::Relaxed) {
                break;
            }
            if maintenance_health.load(Ordering::Relaxed) {
                continue;
            }
            if let Err(error) = invoke(&host, "health_record", "evaluate", json!({})).await {
                tracing::warn!(error = %error, "健康评估失败");
            }
        }
    });
    vec![tick, flush, health, sweep, derive]
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
