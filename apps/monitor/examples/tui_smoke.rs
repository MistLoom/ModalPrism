//! 人工 / 冒烟：短时打开真实 TUI。无控制台 TTY 时会失败并打印原因。
//! 用法：`cargo run -p leaf-monitor --example tui_smoke`
//! 或：`cargo run -p leaf-monitor -- tui --demo --refresh-ms 200`
//! 键位：Tab 切换面板；↑↓/jk 滚动；PgUp/PgDn 翻页；q / Esc 退出。

use std::sync::Arc;
use std::time::Duration;

use ability_core::AbilityHost;
use leaf_monitor::tui::{TuiOpts, run_tui_opts, start_demo_pump};
use leaf_monitor::{MonitorConfig, attach};

#[tokio::main]
async fn main() {
    let host = AbilityHost::new();
    let app = attach(
        &host,
        MonitorConfig {
            log_path: "tui-smoke.jsonl".into(),
            tui_refresh_ms: 200,
            ..MonitorConfig::default()
        },
    );
    let pump = start_demo_pump(250);
    println!("starting DEMO TUI (q to quit); if no console TTY this will error…");
    let app2 = Arc::clone(&app);
    let handle = tokio::task::spawn_blocking(move || {
        run_tui_opts(
            &app2,
            TuiOpts {
                demo: true,
                refresh_ms: Some(200),
            },
        )
    });
    tokio::select! {
        r = handle => {
            pump.store(true, std::sync::atomic::Ordering::SeqCst);
            match r {
                Ok(Ok(())) => println!("TUI exited cleanly"),
                Ok(Err(e)) => eprintln!("TUI error: {e}"),
                Err(e) => eprintln!("join error: {e}"),
            }
        }
        _ = tokio::time::sleep(Duration::from_secs(2)) => {
            eprintln!("TUI still running after 2s (likely waiting for key in real console); aborting smoke");
            pump.store(true, std::sync::atomic::Ordering::SeqCst);
            std::process::exit(0);
        }
    }
    app.shutdown();
}
