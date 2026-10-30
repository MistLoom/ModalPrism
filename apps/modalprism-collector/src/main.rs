//! `modalprism-collector` 的命令行。
//!
//! 启动之外只保留一个对外入口：`tool <verb> [参数] [--json]`。动词表来自
//! `surface::TOOLS`，与 MCP 目录、`GET /tools` 同源；CLI 不自己维护动词清单。
//! 运维入口（`project` / `preconditions`）属于启动侧，用 `api` 子命令透传。

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use assembly::Binds;
use modalprism_collector::boot;
use serde_json::{Map, Value, json};

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
    if let Some(first) = args.first() {
        if first == "tool" {
            args.remove(0);
            return cli_tool(args).await;
        }
        if first == "api" {
            args.remove(0);
            return cli_api(args).await;
        }
        if !first.starts_with("--") {
            return Err(assembly::Error::new(format!(
                "未知参数或动词: {first}（可用: tool <动词>；运维透传: api <路径>；启动开关以 -- 开头）"
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

/// 工具动词：`tool <verb> [--k v ...] [--json]`，位置参数按动词声明顺序填充。
async fn cli_tool(rest: Vec<String>) -> Result<(), assembly::Error> {
    let mut http = "http://127.0.0.1:17880".to_string();
    let mut project: Option<String> = None;
    let mut positional: Vec<String> = Vec::new();
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut pretty = true;
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
                project = Some(value);
            }
            "--help" | "-h" => {
                print!("{}", help());
                return Ok(());
            }
            "--json" => pretty = false,
            other if other.starts_with("--") => {
                let Some(value) = args.next() else {
                    return Err(assembly::Error::new(format!("缺少参数值: {other}")));
                };
                pairs.push((other.trim_start_matches("--").to_string(), value));
            }
            other => positional.push(other.to_string()),
        }
    }
    if positional.is_empty() {
        print!("{}", help());
        return Ok(());
    }
    let verb = positional.remove(0);
    let spec = surface::tool(&verb).ok_or_else(|| {
        assembly::Error::new(format!(
            "未知工具动词: {verb}（可用: {}）",
            surface::tool_ids().join(" / ")
        ))
    })?;
    // 位置参数按声明的参数顺序填充（跳过已用 --k v 给出的同名字段）。
    let mut payload = Map::new();
    for (index, value) in positional.iter().enumerate() {
        let param = spec
            .params
            .iter()
            .filter(|param| {
                !pairs
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case(param.name))
            })
            .nth(index);
        let Some(param) = param else {
            return Err(assembly::Error::new(format!(
                "多余的参数: {value}（{} 只接受 {} 个位置参数）",
                verb,
                spec.params.len()
            )));
        };
        payload.insert(param.name.to_string(), coerce(value, param.kind));
    }
    for (name, value) in &pairs {
        let param = spec
            .params
            .iter()
            .find(|param| param.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| {
                assembly::Error::new(format!(
                    "未知参数: --{name}（{} 接受: {}）",
                    verb,
                    spec.params
                        .iter()
                        .map(|param| param.name)
                        .collect::<Vec<_>>()
                        .join(" / ")
                ))
            })?;
        payload.insert(param.name.to_string(), coerce(value, param.kind));
    }
    for param in spec.params.iter().filter(|param| param.required) {
        if !payload.contains_key(param.name) {
            return Err(assembly::Error::new(format!(
                "缺少必填参数: {}{}",
                param.name,
                spec.params
                    .iter()
                    .find(|item| item.name == param.name)
                    .map(|item| format!("（{}）", item.summary))
                    .unwrap_or_default()
            )));
        }
    }
    let mut request = reqwest::Client::new()
        .post(format!("{http}/api/v1/tools/{verb}"))
        .json(&Value::Object(payload));
    if let Some(project) = project {
        request = request.header("x-project-id", project);
    }
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
    match serde_json::from_str::<Value>(&text) {
        Ok(value) if pretty => println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| text.clone())
        ),
        _ => println!("{text}"),
    }
    if !status.is_success() {
        return Err(assembly::Error::new(format!("API 返回 {status}")));
    }
    Ok(())
}

/// 位置参数的字面值换成 JSON：数组按逗号切成字符串数组，其余按声明类型转换。
fn coerce(value: &str, kind: &str) -> Value {
    match kind {
        "number" => value
            .parse::<f64>()
            .map(|number| json!(number))
            .unwrap_or_else(|_| json!(value)),
        "boolean" => json!(value == "true" || value == "1"),
        "array" => json!(
            value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>()
        ),
        _ => json!(value),
    }
}

/// 运维透传：`api <path> [--method POST] [--body '<json>']`。不新增业务，只走既有端点。
async fn cli_api(rest: Vec<String>) -> Result<(), assembly::Error> {
    let mut http = "http://127.0.0.1:17880".to_string();
    let mut method = "GET".to_string();
    let mut body: Option<Value> = None;
    let mut path: Option<String> = None;
    let mut args = rest.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--http" => {
                http = args
                    .next()
                    .ok_or_else(|| assembly::Error::new("缺少参数值: --http"))?;
            }
            "--method" => {
                method = args
                    .next()
                    .ok_or_else(|| assembly::Error::new("缺少参数值: --method"))?
                    .to_uppercase();
            }
            "--body" => {
                let text = args
                    .next()
                    .ok_or_else(|| assembly::Error::new("缺少参数值: --body"))?;
                body = Some(
                    serde_json::from_str(&text)
                        .map_err(|error| assembly::Error::new(format!("--body 解析失败: {error}")))?,
                );
            }
            other => path = Some(other.to_string()),
        }
    }
    let path = path.ok_or_else(|| assembly::Error::new("用法: modalprism-collector api <path>"))?;
    let url = format!("{http}/api/v1{}", if path.starts_with('/') { path } else { format!("/{path}") });
    let client = reqwest::Client::new();
    let request = match method.as_str() {
        "GET" => client.get(url),
        "POST" => client.post(url).json(&body.unwrap_or(Value::Null)),
        "PATCH" => client.patch(url).json(&body.unwrap_or(Value::Null)),
        "DELETE" => client.delete(url),
        other => return Err(assembly::Error::new(format!("不支持的方法: {other}"))),
    };
    let response = request
        .send()
        .await
        .map_err(|error| assembly::Error::new(format!("领域 API 不可达: {error}")))?;
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

/// `--help`：与动词表同源生成，新增动词不用改这里。
fn help() -> String {
    let mut out = String::from(
        "用法: modalprism-collector tool <动词> [参数…] [--json] [--http <地址>] [--project <id>]\n\n动词：\n",
    );
    for spec in surface::TOOLS {
        out.push_str(&format!("  {:<8} {}\n", spec.id, spec.summary));
        for param in spec.params {
            out.push_str(&format!(
                "      --{:<10} {:<7} {}{}\n",
                param.name,
                param.kind,
                if param.required { "必填 " } else { "     " },
                param.summary
            ));
        }
    }
    out.push_str("\n运维透传: modalprism-collector api <path> [--method GET|POST|PATCH|DELETE] [--body '<json>']\n");
    out
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
