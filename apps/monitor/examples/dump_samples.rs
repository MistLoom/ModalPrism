//! Export live monitor response samples (JSON / CLI / HTTP / fun / graph / stats) for local inspection.
//!
//! Default output: `target/monitor-response-samples.md` under the workspace root.
//! Override with env `LEAF_MONITOR_SAMPLES_OUT=/path/to/file.md`.
//!
//! Run: `cargo run -p leaf-monitor --example dump_samples`

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ability_core::AbilityHost;
use ability_support::monitor::MonitorEvent;
use ability_support::{
    Ability, FnAbility, InstanceId, LoadSpec, MethodId, SourceId, StaticProvider,
};
use frame_core::InvokeRequest;
use leaf_monitor::cli::{MonitorCmd, run_cli};
use leaf_monitor::http::serve_if_enabled;
use leaf_monitor::{HttpConfig, MonitorConfig, attach, fun_call};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

fn provider_ok(id: &str) -> Arc<dyn Ability> {
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
    Arc::new(FnAbility::new(m, |a| async move {
        tokio::time::sleep(Duration::from_millis(250)).await;
        Ok(a)
    }))
}

fn provider_err(id: &str) -> Arc<dyn Ability> {
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
    Arc::new(FnAbility::new(m, |_a| async move {
        Err(ability_support::Error::failed("boom-for-sample"))
    }))
}

async fn http_exchange(addr: &str, method: &str, path: &str, body: Option<&str>) -> String {
    let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
    if let Some(b) = body {
        req.push_str("Content-Type: application/json\r\n");
        req.push_str(&format!("Content-Length: {}\r\n\r\n", b.len()));
        req.push_str(b);
    } else {
        req.push_str("\r\n");
    }
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    String::from_utf8_lossy(&buf).into_owned()
}

fn fmt_cli_chains(chains: &[ability_support::ChainStatus]) -> String {
    let mut s = String::new();
    s.push_str(&format!("{:=<58}\n", " 在途调用链 "));
    if chains.is_empty() {
        s.push_str("(无在途链路)\n");
    }
    for c in chains {
        s.push_str(&format!(
            "{:<38} {} {} span={} running={}\n",
            c.chain_id,
            c.instance,
            c.method.as_deref().unwrap_or("-"),
            c.span,
            c.running
        ));
    }
    s.push_str(&format!("{:=<58}\n", ""));
    s
}

fn fmt_cli_events(evs: &[MonitorEvent], tail: usize) -> String {
    let start = evs.len().saturating_sub(tail);
    let mut s = String::new();
    s.push_str(&format!("{:=<58}\n", " 最近监测事件 "));
    for ev in &evs[start..] {
        s.push_str(&format!(
            "{} [{}] {} {} {}\n",
            ev.at,
            ev.kind,
            ev.phase.as_deref().unwrap_or("-"),
            ev.instance.as_deref().unwrap_or("-"),
            ev.method.as_deref().unwrap_or("-"),
        ));
    }
    s.push_str(&format!("{:=<58}\n", ""));
    s
}

#[tokio::main]
async fn main() {
    let out_path = std::env::var_os("LEAF_MONITOR_SAMPLES_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            workspace_root()
                .join("target")
                .join("monitor-response-samples.md")
        });
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let log = std::env::temp_dir().join(format!("leaf-dump-samples-{}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&log);

    let config = MonitorConfig {
        log_path: log.to_string_lossy().into(),
        http: HttpConfig {
            enabled: true,
            bind: "127.0.0.1:0".into(),
        },
        ..MonitorConfig::default()
    };

    let host = AbilityHost::new();
    let app = attach(&host, config);
    host.install_str("echo", Arc::new(StaticProvider::new(provider_ok("echo"))))
        .await
        .unwrap();
    host.install_str("boom", Arc::new(StaticProvider::new(provider_err("boom"))))
        .await
        .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("echo").unwrap(),
        InstanceId::new("echo").unwrap(),
    ))
    .await
    .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("boom").unwrap(),
        InstanceId::new("boom").unwrap(),
    ))
    .await
    .unwrap();

    let gw = host.clone().into_gateway();
    let inflight = {
        let gw = Arc::clone(&gw);
        tokio::spawn(async move {
            gw.invoke(InvokeRequest::new(
                InstanceId::new("echo").unwrap(),
                MethodId::new("run").unwrap(),
                json!({"x": 1}),
            ))
            .await
        })
    };
    let mut chains_busy = Vec::new();
    for _ in 0..200 {
        chains_busy = app.chains();
        if !chains_busy.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let chains_busy_json =
        serde_json::to_string_pretty(&json!({ "chains": &chains_busy })).unwrap();
    let _ = inflight.await.unwrap();

    let _ = gw
        .invoke(InvokeRequest::new(
            InstanceId::new("boom").unwrap(),
            MethodId::new("run").unwrap(),
            json!({}),
        ))
        .await;

    tokio::time::sleep(Duration::from_millis(700)).await;

    let events = app.recent_events();
    let pick = |kind: &str, phase: Option<&str>| -> Option<MonitorEvent> {
        events
            .iter()
            .rev()
            .find(|e| {
                e.kind == kind
                    && match phase {
                        Some(p) => e.phase.as_deref() == Some(p),
                        None => true,
                    }
            })
            .cloned()
    };
    let ev_before = pick("invoke", Some("before"));
    let ev_after = pick("invoke", Some("after"));
    let ev_error = pick("invoke", Some("error"));
    let ev_life = pick("lifecycle", None);
    let ev_host = pick("host", None);

    let chains_empty = app.chains();
    let chains_empty_json =
        serde_json::to_string_pretty(&json!({ "chains": &chains_empty })).unwrap();

    let cli_chains_busy = fmt_cli_chains(&chains_busy);
    let cli_chains = fmt_cli_chains(&chains_empty);
    let cli_events = fmt_cli_events(&events, 8);
    let log_lines = app.tail_log(5).unwrap_or_default();
    let cli_log = {
        let mut s = String::new();
        for l in &log_lines {
            s.push_str(l);
            s.push('\n');
        }
        s
    };
    let _ = run_cli(&app, MonitorCmd::Chains);

    let addr = serve_if_enabled(Arc::clone(&app))
        .await
        .unwrap()
        .expect("http on");
    let addr_s = addr.to_string();
    tokio::time::sleep(Duration::from_millis(80)).await;
    let http_chains = http_exchange(&addr_s, "GET", "/monitor/chains", None).await;
    let http_events =
        http_exchange(&addr_s, "POST", "/monitor/events", Some(r#"{"tail":5}"#)).await;
    let http_log = http_exchange(&addr_s, "POST", "/monitor/log", Some(r#"{"tail":3}"#)).await;

    let fun_chains = fun_call(&app, "monitor.chains", json!({})).await.unwrap();
    let fun_events = fun_call(&app, "monitor.events", json!({"tail": 3}))
        .await
        .unwrap();
    let fun_log = fun_call(&app, "monitor.log", json!({"tail": 2}))
        .await
        .unwrap();
    let fun_err = fun_call(&app, "monitor.nope", json!({}))
        .await
        .unwrap_err()
        .to_string();
    let fun_graph = fun_call(&app, "monitor.graph", json!({})).await.unwrap();
    let http_stats = http_exchange(&addr_s, "GET", "/monitor/stats", None).await;
    let http_graph = http_exchange(&addr_s, "GET", "/monitor/graph", None).await;
    // Force some drops for a non-zero sample if still zero
    if app.stats().dropped == 0 {
        for i in 0..64u64 {
            ability_support::monitor::emit(
                ability_support::monitor::MonitorEvent::new("invoke", i).phase("noise"),
            );
        }
    }
    let fun_stats = fun_call(&app, "monitor.stats", json!({})).await.unwrap();

    let tui_header = " LEAF monitor · DEMO  refresh=200ms  last=0ms ago  focus=events  | Tab 焦点  ↑↓/jk 滚动  PgUp/PgDn 翻页  q 退出";
    let tui_chain_row = format!(
        "{:<22}  {:<16}  {:<12}  span={}  {}",
        chains_busy
            .first()
            .map(|c| {
                let id = c.chain_id.as_str();
                if id.len() > 22 { &id[..22] } else { id }
            })
            .unwrap_or("a1b2c3d4-e5f6-7890-ab"),
        "echo",
        "run",
        0,
        "running"
    );
    let tui_event_row = if let Some(ev) = ev_before.as_ref() {
        let at = ev.at;
        let secs = at / 1000;
        let ms = at % 1000;
        let s = secs % 60;
        let m = (secs / 60) % 60;
        let h = (secs / 3600) % 24;
        let clock = format!("{h:02}:{m:02}:{s:02}.{ms:03}");
        let cid = ev.chain_id.as_deref().unwrap_or("-");
        let short = if cid.len() > 8 { &cid[..8] } else { cid };
        format!(
            "{clock}  <age>ms  [invoke] before  {}  {}  chain={short}…  span={}",
            ev.instance.as_deref().unwrap_or("-"),
            ev.method.as_deref().unwrap_or("-"),
            ev.span.unwrap_or(0)
        )
    } else {
        "HH:MM:SS.mmm  12ms  [invoke] before  echo  run  chain=a1b2c3d4…  span=0".into()
    };

    let mut md = String::new();
    md.push_str("# Leaf monitor response samples\n\n");
    md.push_str("Generated by `cargo run -p leaf-monitor --example dump_samples`.\n\n");

    md.push_str("## 1. MonitorEvent JSON\n\n");
    for (label, ev) in [
        ("invoke before", &ev_before),
        ("invoke after", &ev_after),
        ("invoke error", &ev_error),
        ("lifecycle", &ev_life),
        ("host", &ev_host),
    ] {
        md.push_str(&format!("### {label}\n\n```json\n"));
        match ev {
            Some(e) => md.push_str(&serde_json::to_string_pretty(e).unwrap()),
            None => md.push_str(&format!("{{\"error\":\"no {label} event captured\"}}")),
        }
        md.push_str("\n```\n\n");
    }

    md.push_str("## 2. chains 快照\n\n### 有在途时\n\n```json\n");
    md.push_str(&chains_busy_json);
    md.push_str("\n```\n\n### 空列表时\n\n```json\n");
    md.push_str(&chains_empty_json);
    md.push_str("\n```\n\n");

    md.push_str(
        "## 3. CLI 打印文本（与 `cli.rs` / `run_cli` 同形）\n\n### Chains（有在途）\n\n```text\n",
    );
    md.push_str(&cli_chains_busy);
    md.push_str("```\n\n### Chains（空）\n\n```text\n");
    md.push_str(&cli_chains);
    md.push_str("```\n\n### Events（tail=8）\n\n```text\n");
    md.push_str(&cli_events);
    md.push_str("```\n\n### Log\n\n```text\n");
    md.push_str(&cli_log);
    md.push_str("```\n\n");

    md.push_str("## 4. HTTP 状态码 + body\n\n### GET /monitor/chains\n\n```http\n");
    md.push_str(&http_chains);
    md.push_str("\n```\n\n### POST /monitor/events\n\n```http\n");
    md.push_str(&http_events);
    md.push_str("\n```\n\n### POST /monitor/log\n\n```http\n");
    md.push_str(&http_log);
    md.push_str("\n```\n\n");

    md.push_str("## 5. fun_call\n\n### monitor.chains\n\n```json\n");
    md.push_str(&serde_json::to_string_pretty(&fun_chains).unwrap());
    md.push_str("\n```\n\n### monitor.events\n\n```json\n");
    md.push_str(&serde_json::to_string_pretty(&fun_events).unwrap());
    md.push_str("\n```\n\n### monitor.log\n\n```json\n");
    md.push_str(&serde_json::to_string_pretty(&fun_log).unwrap());
    md.push_str("\n```\n\n### monitor.nope 错误\n\n```text\n");
    md.push_str(&fun_err);
    md.push_str("\n```\n\n");

    md.push_str("## 5b. monitor.stats / graph\n\n### fun monitor.stats\n\n```json\n");
    md.push_str(&serde_json::to_string_pretty(&fun_stats).unwrap());
    md.push_str("\n```\n\n### fun monitor.graph\n\n```json\n");
    md.push_str(&serde_json::to_string_pretty(&fun_graph).unwrap());
    md.push_str("\n```\n\n### GET /monitor/stats\n\n```http\n");
    md.push_str(&http_stats);
    md.push_str("\n```\n\n### GET /monitor/graph\n\n```http\n");
    md.push_str(&http_graph);
    md.push_str("\n```\n\n");

    md.push_str("## 6. JSONL 原文（最多 2 行）\n\n```jsonl\n");
    for line in log_lines.iter().take(2) {
        md.push_str(line);
        md.push('\n');
    }
    md.push_str("```\n\n");

    md.push_str("## 7. TUI 屏区字符串形态（DEMO，与 `tui.rs` draw 一致）\n\n");
    md.push_str(&format!("- 标题行：`{tui_header}`\n"));
    md.push_str(
        "- chains 面板标题：`▶ monitor · chains (在途)`（焦点）或 `monitor · chains (在途)`\n",
    );
    md.push_str("- chains 摘要：`在途链路=N  | 字段: chain_id  instance  method  span  running`\n");
    md.push_str(&format!("- chains 数据行：`{tui_chain_row}`\n"));
    md.push_str("- chains 空：`(无在途链路 · 等待 Host invoke)`\n");
    md.push_str("- events 面板标题：`▶ monitor · events (流)` / `monitor · events (流)`\n");
    md.push_str(&format!("- events 典型一行：`{tui_event_row}`\n"));
    md.push_str("- 键位：Tab；↑↓/jk；PgUp/PgDn；Home/End；q/Esc\n");

    std::fs::write(&out_path, md.as_bytes()).unwrap();
    println!("wrote {}", out_path.display());
    app.shutdown();
    let _ = std::fs::remove_file(&log);
}
