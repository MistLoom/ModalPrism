//! `modalprism-collector` 的命令行。

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use assembly::Binds;
use modalprism_collector::boot;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), assembly::Error> {
    let mut data = PathBuf::from("var");
    let mut config = PathBuf::from("config");
    let mut http: SocketAddr = "127.0.0.1:17880".parse().expect("http");
    let mut proxy: SocketAddr = "127.0.0.1:17881".parse().expect("proxy");
    let mut push: SocketAddr = "127.0.0.1:17882".parse().expect("push");
    let mut args = env::args().skip(1);
    while let Some(flag) = args.next() {
        let Some(value) = args.next() else {
            return Err(assembly::Error::new(format!("缺少参数值: {flag}")));
        };
        match flag.as_str() {
            "--data" => data = PathBuf::from(value),
            "--config" => config = PathBuf::from(value),
            "--http" => http = parse_addr(&value)?,
            "--proxy" => proxy = parse_addr(&value)?,
            "--push" => push = parse_addr(&value)?,
            _ => return Err(assembly::Error::new(format!("未知参数: {flag}"))),
        }
    }
    let running = boot(&data, &config, Binds { http, proxy, push }).await?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            result.map_err(|source| assembly::Error::with_source("信号", source))?;
        }
        result = wait_term() => {
            result?;
        }
    }
    running.shutdown().await
}

fn parse_addr(value: &str) -> Result<SocketAddr, assembly::Error> {
    value
        .parse()
        .map_err(|_| assembly::Error::new(format!("监听地址无效: {value}")))
}

async fn wait_term() -> Result<(), assembly::Error> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate())
            .map_err(|source| assembly::Error::with_source("信号", source))?;
        term.recv().await;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::future::pending::<()>().await;
        Ok(())
    }
}
