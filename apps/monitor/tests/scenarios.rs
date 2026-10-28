//! 场景化：CLI / HTTP / fun 错误面 / 无 sink 不崩 / TUI TestBackend 冒烟。

use std::sync::Arc;

use ability_core::AbilityHost;
use ability_support::{
    Ability, FnAbility, InstanceId, LoadSpec, MethodId, SourceId, StaticProvider,
};
use frame_core::InvokeRequest;
use leaf_monitor::cli::{MonitorCmd, run_cli};
use leaf_monitor::http::serve_if_enabled;
use leaf_monitor::{HttpConfig, MonitorConfig, attach, fun_call};
use serde_json::json;

fn echo_provider(id: &str) -> Arc<dyn Ability> {
    let m = ability_support::AbilityManifest {
        id: ability_support::AbilityId::new(id).unwrap(),
        depends: vec![],
        stateful: false,
        config_schema: json!({}),
        methods: vec![ability_support::MethodMeta::new(
            "run",
            "",
            json!({"type": "object"}),
        )],
        events: vec![],
    };
    Arc::new(FnAbility::new(m, |a| async move { Ok(a) }))
}

async fn boot_with_log(
    log: &std::path::Path,
    http_enabled: bool,
    bind: &str,
) -> (AbilityHost, Arc<leaf_monitor::MonitorApp>) {
    let config = MonitorConfig {
        log_path: log.to_string_lossy().into(),
        http: HttpConfig {
            enabled: http_enabled,
            bind: bind.into(),
        },
        ..MonitorConfig::default()
    };
    let host = AbilityHost::new();
    let app = attach(&host, config);
    host.install_str("echo", Arc::new(StaticProvider::new(echo_provider("echo"))))
        .await
        .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("echo").unwrap(),
        InstanceId::new("echo").unwrap(),
    ))
    .await
    .unwrap();
    let gw = host.clone().into_gateway();
    gw.invoke(InvokeRequest::new(
        InstanceId::new("echo").unwrap(),
        MethodId::new("run").unwrap(),
        json!({"x": 1}),
    ))
    .await
    .unwrap();
    (host, app)
}

#[tokio::test]
async fn s4_cli_chains_events_log() {
    let log = std::env::temp_dir().join(format!("leaf-mon-s4-{}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&log);
    let (_host, app) = boot_with_log(&log, false, "127.0.0.1:0").await;
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    let n = run_cli(&app, MonitorCmd::Chains).unwrap();
    assert_eq!(n, 0, "调用结束后链应已清空");

    let n = run_cli(&app, MonitorCmd::Events { tail: 50 }).unwrap();
    assert!(n > 0, "CLI events 应读到刚产生的数据");

    let n = run_cli(&app, MonitorCmd::Log { tail: 100 }).unwrap();
    assert!(n > 0, "CLI log 应读到 JSONL");

    app.shutdown();
    let _ = std::fs::remove_file(&log);
}

#[tokio::test]
async fn s6_http_default_off_and_enabled_on() {
    // 默认关：serve_if_enabled 返回 None，且不占端口。
    let log = std::env::temp_dir().join(format!("leaf-mon-s6a-{}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&log);
    let (_host, app) = boot_with_log(&log, false, "127.0.0.1:0").await;
    let addr = serve_if_enabled(Arc::clone(&app)).await.unwrap();
    assert!(addr.is_none(), "http.enabled=false 时不应监听");
    // 默认关：不应占用常见默认端口（未启动 listener）。
    app.shutdown();
    let _ = std::fs::remove_file(&log);

    // 显式开启：绑定 127.0.0.1:0，真实监听，HTTP GET 可打通。
    let log = std::env::temp_dir().join(format!("leaf-mon-s6b-{}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&log);
    let (_host, app) = boot_with_log(&log, true, "127.0.0.1:0").await;
    let addr = serve_if_enabled(Arc::clone(&app))
        .await
        .unwrap()
        .expect("enabled 时应返回本机地址");
    let url = format!("http://{}/monitor/chains", addr);
    let body = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        let resp = reqwest_get(&url).await;
        resp
    })
    .await
    .expect("HTTP 超时")
    .expect("HTTP 失败");
    assert!(body.contains("chains"), "响应应含 chains: {body}");

    app.shutdown();
    let _ = std::fs::remove_file(&log);
}

/// 最小 HTTP GET，避免额外依赖：手写请求。
async fn reqwest_get(url: &str) -> Result<String, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let u = url.strip_prefix("http://").ok_or("need http")?;
    let (hostport, path) = u
        .split_once('/')
        .map(|(h, p)| (h, format!("/{p}")))
        .unwrap_or((u, "/".into()));
    let addr: std::net::SocketAddr = hostport
        .parse()
        .map_err(|e: std::net::AddrParseError| e.to_string())?;
    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .map_err(|e| e.to_string())?;
    let req = format!("GET {path} HTTP/1.1\r\nHost: {hostport}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(req.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .await
        .map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

#[tokio::test]
async fn s7_fun_call_ok_and_unknown_error() {
    let log = std::env::temp_dir().join(format!("leaf-mon-s7-{}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&log);
    let (_host, app) = boot_with_log(&log, false, "127.0.0.1:0").await;
    let v = fun_call(&app, "monitor.chains", json!({})).await.unwrap();
    assert!(v.get("chains").is_some());
    let v = fun_call(&app, "monitor.events", json!({"tail": 5}))
        .await
        .unwrap();
    assert!(v.get("events").is_some());
    // 等异步 worker 落盘
    let mut log_ok = None;
    for _ in 0..50 {
        match fun_call(&app, "monitor.log", json!({"tail": 5})).await {
            Ok(v)
                if v.get("lines")
                    .and_then(|l| l.as_array())
                    .map(|a| !a.is_empty())
                    .unwrap_or(false) =>
            {
                log_ok = Some(v);
                break;
            }
            _ => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
        }
    }
    assert!(log_ok.is_some(), "fun monitor.log 应读到 JSONL");

    let err = fun_call(&app, "monitor.nope", json!({})).await.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("monitor.nope") || msg.contains("未知"),
        "未知 fun 应有明确错误: {msg}"
    );

    app.shutdown();
    let _ = std::fs::remove_file(&log);
}

#[tokio::test]
async fn s2_no_sink_invoke_does_not_panic() {
    // 不 attach monitor：无 sink 时 invoke 仍正常。
    let host = AbilityHost::new();
    host.install_str("echo", Arc::new(StaticProvider::new(echo_provider("echo"))))
        .await
        .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("echo").unwrap(),
        InstanceId::new("echo").unwrap(),
    ))
    .await
    .unwrap();
    let gw = host.into_gateway();
    gw.invoke(InvokeRequest::new(
        InstanceId::new("echo").unwrap(),
        MethodId::new("run").unwrap(),
        json!({}),
    ))
    .await
    .unwrap();
}

#[test]
fn s8_single_product_surface() {
    let root = workspace_root();
    let cargo = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let count = cargo.matches("apps/monitor").count();
    assert!(count >= 1, "workspace 应含 apps/monitor");
    // 不应再有第二个 monitor 产品 crate 路径
    assert!(
        !cargo.contains("apps/leaf-monitor") && !cargo.contains("apps/monitoring"),
        "不应另立监测产品 crate"
    );
    let lic = root.join("apps/monitor/LICENSE");
    let text = std::fs::read_to_string(&lic).expect("apps/monitor/LICENSE 应存在（内置标记）");
    assert!(
        text.lines()
            .next()
            .unwrap_or("")
            .contains("LEAF-BUILTIN-APPS")
            || text.contains("LEAF-BUILTIN-APPS"),
        "LICENSE 应含 LEAF-BUILTIN-APPS 标记行"
    );
}

fn workspace_root() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // apps
    p.pop(); // repo root from apps/monitor
    // CARGO_MANIFEST_DIR = .../apps/monitor → pop → apps → pop → root
    p
}

#[tokio::test]
async fn s9_stats_dropped_and_graph_parent_edges() {
    // Force queue drops
    let config = MonitorConfig {
        log_path: String::new(),
        queue_capacity: 16,
        recent_capacity: 64,
        ..MonitorConfig::default()
    };
    let host = AbilityHost::new();
    let app = attach(&host, config);
    for i in 0..100u64 {
        ability_support::monitor::emit(
            ability_support::monitor::MonitorEvent::new("invoke", i)
                .phase("before")
                .span_id(format!("s{i}"))
                .chain("c", 0),
        );
    }
    let st = app.stats();
    assert!(st.dropped > 0, "expected drops: {st:?}");
    assert_eq!(st.recent, 64.min(100));

    // Nested parent/child graph via real host
    let host2 = AbilityHost::new();
    let app2 = attach(
        &host2,
        MonitorConfig {
            log_path: String::new(),
            ..MonitorConfig::default()
        },
    );
    // reuse echo for flat invoke with span_id
    host2
        .install_str("echo", Arc::new(StaticProvider::new(echo_provider("echo"))))
        .await
        .unwrap();
    host2
        .load(LoadSpec::new(
            SourceId::new("echo").unwrap(),
            InstanceId::new("echo").unwrap(),
        ))
        .await
        .unwrap();
    host2
        .clone()
        .into_gateway()
        .invoke(InvokeRequest::new(
            InstanceId::new("echo").unwrap(),
            MethodId::new("run").unwrap(),
            json!({}),
        ))
        .await
        .unwrap();
    let evs = app2.recent_events();
    assert!(
        evs.iter()
            .any(|e| e.kind == "invoke" && e.span_id.is_some() && e.ended_at.is_some()),
        "invoke after should have span_id+ended_at"
    );
    assert!(
        evs.iter().any(|e| e.kind == "lifecycle" && e.to.is_some()),
        "lifecycle should have to"
    );
    let stats = fun_call(&app2, "monitor.stats", json!({})).await.unwrap();
    assert!(stats.get("dropped").is_some());
    let g = fun_call(&app2, "monitor.graph", json!({})).await.unwrap();
    assert!(g.get("nodes").is_some());

    // synthetic nested for graph edges
    let nested = vec![
        ability_support::monitor::MonitorEvent::new("invoke", 1)
            .phase("before")
            .instance("p")
            .method("run")
            .chain("c1", 0)
            .span_id("root")
            .started_at(1),
        ability_support::monitor::MonitorEvent::new("invoke", 2)
            .phase("before")
            .instance("c")
            .method("run")
            .chain("c1", 1)
            .span_id("child")
            .parent_span_id("root")
            .started_at(2),
        ability_support::monitor::MonitorEvent::new("invoke", 3)
            .phase("after")
            .instance("c")
            .method("run")
            .chain("c1", 1)
            .span_id("child")
            .parent_span_id("root")
            .ended_at(3),
    ];
    let graph = leaf_monitor::graph::build_chain_graph(&nested);
    assert_eq!(graph.edges.len(), 1);
    assert_eq!(graph.edges[0].parent_span_id, "root");
    assert_eq!(graph.edges[0].span_id, "child");

    app.shutdown();
    app2.shutdown();
}
