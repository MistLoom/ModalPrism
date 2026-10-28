//! `monitor` 命令行：离线日志查询 + 实时 TUI。
//!
//! 在途链路 / 在线查询面需要附着到运行中的宿主（进程内），由入口应用
//! 装配（见 lib `attach` / `run_cli` / `run_tui` / `serve_if_enabled`）。
//! 本 bin 提供：
//! - `monitor log --file … --tail N`：无需宿主的离线 JSONL 查询
//! - `monitor tui [--demo]`：独立实时视图（默认 demo 泵，便于无完整 Host 时观察）

use std::sync::Arc;
use std::time::Duration;

use ability_core::AbilityHost;
use ability_support::{
    Ability, FnAbility, InstanceId, LoadSpec, MethodId, SourceId, StaticProvider,
};
use clap::{Parser, Subcommand};
use frame_core::InvokeRequest;
use leaf_monitor::tail_log_file;
use leaf_monitor::tui::{TuiOpts, run_tui_opts, start_demo_pump};
use leaf_monitor::{MonitorConfig, attach};
use serde_json::json;

#[derive(Parser)]
#[command(name = "monitor", about = "LEAF 监测查询（离线日志 / 实时 TUI）")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 尾读监测 JSONL 日志
    Log {
        #[arg(long, default_value = "monitor.jsonl")]
        file: String,
        #[arg(long, default_value_t = 64)]
        tail: usize,
    },
    /// 打开实时 TUI 视图（双面板：在途链 + 事件流）
    Tui {
        /// 关闭 demo 泵（默认开启 demo：事件泵 + 慢调用，标题栏标明 DEMO）
        #[arg(long, default_value_t = false)]
        no_demo: bool,
        /// 刷新间隔毫秒（覆盖 monitor.yml / 默认 500）
        #[arg(long)]
        refresh_ms: Option<u64>,
        /// 配置文件路径（默认 monitor.yml，不存在则用默认值）
        #[arg(long)]
        config: Option<String>,
        /// Demo 事件泵间隔毫秒
        #[arg(long, default_value_t = 400)]
        pump_ms: u64,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Log { file, tail } => match tail_log_file(&file, tail) {
            Ok(lines) => {
                for l in lines {
                    println!("{l}");
                }
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        },
        Commands::Tui {
            no_demo,
            refresh_ms,
            config,
            pump_ms,
        } => {
            let use_demo = !no_demo;
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(async move {
                if let Err(e) = run_tui_cmd(use_demo, refresh_ms, config, pump_ms).await {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            });
        }
    }
}

async fn run_tui_cmd(
    demo: bool,
    refresh_ms: Option<u64>,
    config: Option<String>,
    pump_ms: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut cfg = match config {
        Some(p) => MonitorConfig::load(Some(std::path::Path::new(&p))),
        None => MonitorConfig::load(None),
    };
    // 独立 TUI 默认不抢占业务 JSONL；可在 yml 里改
    if cfg.log_path == "monitor.jsonl" {
        cfg.log_path = "monitor-tui.jsonl".into();
    }
    if let Some(ms) = refresh_ms {
        cfg.tui_refresh_ms = ms;
    }

    let host = AbilityHost::new();
    let app = attach(&host, cfg);

    let pump_stop = if demo {
        // 装一个慢 echo，后台循环 invoke，让 chains 面板也有实时数据
        let echo = echo_provider("echo");
        host.install_str("echo", Arc::new(StaticProvider::new(echo)))
            .await?;
        host.load(LoadSpec::new(
            SourceId::new("echo").unwrap(),
            InstanceId::new("echo").unwrap(),
        ))
        .await?;
        let gw = host.clone().into_gateway();
        let stop_inv = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_inv2 = Arc::clone(&stop_inv);
        tokio::spawn(async move {
            while !stop_inv2.load(std::sync::atomic::Ordering::SeqCst) {
                let _ = gw
                    .invoke(InvokeRequest::new(
                        InstanceId::new("echo").unwrap(),
                        MethodId::new("run").unwrap(),
                        json!({"demo": true}),
                    ))
                    .await;
                tokio::time::sleep(Duration::from_millis(800)).await;
            }
        });
        let pump = start_demo_pump(pump_ms);
        Some((pump, stop_inv))
    } else {
        None
    };

    let app2 = Arc::clone(&app);
    let opts = TuiOpts { demo, refresh_ms };
    let handle = tokio::task::spawn_blocking(move || run_tui_opts(&app2, opts));
    let result = handle.await?;
    if let Some((pump, stop_inv)) = pump_stop {
        pump.store(true, std::sync::atomic::Ordering::SeqCst);
        stop_inv.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    app.shutdown();
    result?;
    Ok(())
}

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
    // 停留足够久，让 TUI 能刷到在途链
    Arc::new(FnAbility::new(m, |a| async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        Ok(a)
    }))
}
