//! `modalprism-console` 的命令行。
//!
//! 默认只监听环回地址，与采集进程同机运行；前端与接口同源，不需要额外跨域配置。

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use modalprism_console::{ops::Ops, router};

#[tokio::main]
async fn main() {
    init_log();
    if let Err(message) = run().await {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

fn init_log() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

async fn run() -> Result<(), String> {
    let mut bind: SocketAddr = "127.0.0.1:17890".parse().expect("http");
    let mut ops = "http://127.0.0.1:17880".to_string();
    let mut web = default_web();
    let mut config_dir = PathBuf::from("config");
    let mut args = env::args().skip(1);
    while let Some(flag) = args.next() {
        let Some(value) = args.next() else {
            return Err(format!("缺少参数值: {flag}"));
        };
        match flag.as_str() {
            "--http" | "--bind" => {
                bind = value
                    .parse()
                    .map_err(|_| format!("监听地址无效: {value}"))?
            }
            "--ops" => ops = value,
            "--web" => web = PathBuf::from(value),
            "--config" => config_dir = PathBuf::from(value),
            _ => return Err(format!("未知参数: {flag}")),
        }
    }
    let ops = Ops::new(&ops).map_err(|error| error.to_string())?;
    let endpoint = ops.base().to_string();
    let app = router(ops, web.clone(), config_dir.clone());
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|error| format!("监听 {bind} 失败: {error}"))?;
    println!(
        "modalprism-console: http=http://{bind} ops={endpoint} web={} config={}",
        web.display(),
        config_dir.display()
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await
        .map_err(|error| format!("服务退出: {error}"))
}

/// 产物目录默认取可执行文件旁的 `web`，否则回落到源码目录。
fn default_web() -> PathBuf {
    if let Ok(exe) = env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let candidate = dir.join("web");
        if candidate.is_dir() {
            return candidate;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web/dist")
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}
