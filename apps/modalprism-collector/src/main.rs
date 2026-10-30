//! `modalprism-collector` 的命令行。
//!
//! 启动之外提供一等公民动词（search / fetch / parse / project），动词走
//! 本机回环的领域 API，不重复实现业务逻辑。

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use assembly::Binds;
use modalprism_collector::boot;

#[tokio::main]
async fn main() {
    init_log();
    if let Err(error) = run().await {
        eprintln!("{error}");
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

async fn run() -> Result<(), assembly::Error> {
    let mut args: Vec<String> = env::args().skip(1).collect();
    // 一等公民动词：面向既有运行实例（默认 http://127.0.0.1:17880）。
    // 其余参数一律是服务端开关（--data/--config/--http/--proxy/--push）；
    // 只有既非动词也非开关的裸参数才报错，避免把 `--data` 当成未知动词。
    if let Some(first) = args.first() {
        if matches!(
            first.as_str(),
            "search" | "fetch" | "parse" | "project" | "preconditions"
        ) {
            let verb = args.remove(0);
            return cli_verb(&verb, args).await;
        }
        if !first.starts_with("--") {
            return Err(assembly::Error::new(format!(
                "未知参数或动词: {first}（可用动词: search / fetch / parse / project / preconditions；服务端开关以 -- 开头）"
            )));
        }
    }
    let running = serve_from(args).await?;
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

async fn serve_from(rest: Vec<String>) -> Result<modalprism_collector::Running, assembly::Error> {
    let mut data = PathBuf::from("var");
    let mut config = PathBuf::from("config");
    let mut http: SocketAddr = "127.0.0.1:17880".parse().expect("http");
    let mut proxy: SocketAddr = "127.0.0.1:17881".parse().expect("proxy");
    let mut push: SocketAddr = "127.0.0.1:17882".parse().expect("push");
    let mut args = rest.into_iter();
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
    boot(&data, &config, Binds { http, proxy, push }).await
}

/// CLI 动词：调用本机回环领域 API 并原样打印 JSON。
async fn cli_verb(verb: &str, rest: Vec<String>) -> Result<(), assembly::Error> {
    let mut http = "http://127.0.0.1:17880".to_string();
    let mut positional: Vec<String> = Vec::new();
    let mut extra: Vec<(String, String)> = Vec::new();
    let mut args = rest.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--http" => {
                let Some(value) = args.next() else {
                    return Err(assembly::Error::new("缺少参数值: --http"));
                };
                http = value;
            }
            "--project" => {
                let Some(value) = args.next() else {
                    return Err(assembly::Error::new("缺少参数值: --project"));
                };
                extra.push(("projectId".into(), value));
            }
            "--engine" => {
                let Some(value) = args.next() else {
                    return Err(assembly::Error::new("缺少参数值: --engine"));
                };
                extra.push(("engine".into(), value));
            }
            other => positional.push(other.to_string()),
        }
    }
    let client = reqwest::Client::new();
    let (method, path, body): (&str, String, Option<serde_json::Value>) = match verb {
        "search" => {
            let query = positional.join(" ");
            if query.is_empty() {
                return Err(assembly::Error::new(
                    "用法: modalprism-collector search <关键词> [--engine <id>]",
                ));
            }
            let mut payload = serde_json::json!({ "query": query });
            if let Some((_, project)) = extra.iter().find(|(key, _)| key == "projectId") {
                payload["projectId"] = serde_json::json!(project);
            }
            // --engine <id>
            let engine = positional_extra(&extra, "engine");
            if let Some(engine) = engine {
                payload["engine"] = serde_json::json!(engine);
            }
            ("POST", "/api/v1/search".into(), Some(payload))
        }
        "fetch" => {
            let url = positional
                .first()
                .ok_or_else(|| assembly::Error::new("用法: modalprism-collector fetch <url>"))?;
            (
                "POST",
                "/api/v1/fetch".into(),
                Some(serde_json::json!({ "url": url })),
            )
        }
        "parse" => {
            let target = positional
                .first()
                .ok_or_else(|| assembly::Error::new("用法: modalprism-collector parse <url>"))?;
            (
                "POST",
                "/api/v1/parse".into(),
                Some(serde_json::json!({ "url": target })),
            )
        }
        "project" => match positional.first().map(String::as_str) {
            Some("list") => ("GET", "/api/v1/projects".into(), None),
            Some("create") => {
                let name = positional.get(1).ok_or_else(|| {
                    assembly::Error::new("用法: modalprism-collector project create <名称>")
                })?;
                (
                    "POST",
                    "/api/v1/projects".into(),
                    Some(serde_json::json!({ "name": name })),
                )
            }
            Some("resources") => {
                let id = positional.get(1).ok_or_else(|| {
                    assembly::Error::new("用法: modalprism-collector project resources <projectId>")
                })?;
                ("GET", format!("/api/v1/projects/{id}/resources"), None)
            }
            other => {
                return Err(assembly::Error::new(format!(
                    "未知 project 子命令: {:?}（可用: list / create / resources）",
                    other
                )));
            }
        },
        "preconditions" => ("GET", "/api/v1/preconditions".into(), None),
        other => return Err(assembly::Error::new(format!("未知动词: {other}"))),
    };
    let request = match (method, body) {
        ("GET", _) => client.get(format!("{http}{path}")),
        ("POST", Some(payload)) => client.post(format!("{http}{path}")).json(&payload),
        ("POST", None) => client.post(format!("{http}{path}")),
        _ => unreachable!("动词方法已枚举"),
    };
    let response = request.send().await.map_err(|error| {
        assembly::Error::new(format!(
            "领域 API 不可达（先启动 modalprism-collector 或用 --http 指定地址）: {error}"
        ))
    })?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| assembly::Error::new(error.to_string()))?;
    println!("{text}");
    if !status.is_success() {
        return Err(assembly::Error::new(format!("API 返回 {status}")));
    }
    Ok(())
}

fn positional_extra(extra: &[(String, String)], key: &str) -> Option<String> {
    extra
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.clone())
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
