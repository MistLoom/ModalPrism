//! `modalprism-collector` 的命令行。
//!
//! 启动之外只保留一个对外入口：`tool <verb> [参数] [--json]`。动词表来自
//! `surface::TOOLS`，与 MCP 目录、`GET /tools` 同源；CLI 不自己维护动词清单。
//! 运维入口（`project` / `preconditions`）属于启动侧，用 `api` 子命令透传。

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use assembly::Binds;
use modalprism_collector::{BootOpts, boot, boot_opts};
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
    if let Some(first) = args.first().cloned() {
        match first.as_str() {
            "serve" => {
                args.remove(0);
            }
            "once" | "search" | "ls" | "read" | "fetch" | "catalog" => {
                args.remove(0);
                return cli_mp(&first, args).await;
            }
            "agent" => {
                args.remove(0);
                return cli_agent(args).await;
            }
            "plan" => {
                args.remove(0);
                return cli_plan(args).await;
            }
            "strategy" => {
                args.remove(0);
                return cli_strategy(args).await;
            }
            "help" | "-h" | "--help" => {
                print!("{}", mp_help());
                return Ok(());
            }
            "tool" => {
                args.remove(0);
                return cli_tool(args).await;
            }
            "api" => {
                args.remove(0);
                return cli_api(args).await;
            }
            // plan.* 等工具动词维持旧的 HTTP 工具面行为：运维契约不变。
            verb if surface::tool(verb).is_some() => {
                return cli_tool(args).await;
            }
            flag if !flag.starts_with("--") => {
                return Err(assembly::Error::new(format!(
                    "未知子命令或动词: {flag}（mp 子命令: serve / once / search / ls / read / fetch / agent / plan；工具动词透传: tool <动词>；运维透传: api <路径>；启动开关以 -- 开头）"
                )));
            }
            _ => {}
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
    // `--file` 只在 CLI 入口开放：调用者与进程同主体，本地文件读取由领域 API 的
    // 统一摄入函数完成（摄入根 <data_dir>/inbox），MCP / 其它 HTTP 调用方不带此参数。
    let file_value = pairs
        .iter()
        .position(|(name, _)| name.eq_ignore_ascii_case("file"))
        .map(|index| pairs.remove(index).1);
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
    if let Some(path) = file_value {
        payload.insert("file".to_string(), json!(path));
    }
    let mut request = reqwest::Client::new()
        .post(format!("{http}/api/v1/tools/{verb}"))
        .header("x-modalprism-surface", "cli")
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
    let path = path.ok_or_else(|| assembly::Error::new("用法: mp api <path>"))?;
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

/// mp 一级短命令 → 工具动词：`catalog` 是 `ls` 的别名。
fn mp_verb(name: &str) -> &str {
    if name == "catalog" {
        "ls"
    } else {
        name
    }
}

/// mp plan 一级子命令（切片 2）：计划动词上同一套 CLI 与 MCP 主路径。
///
/// `mp plan ls|get|new|set|pause|resume|run`，别名 `list|create|update|continue`。
/// 执行方式与切片 1 相同：附加到运行中的 collector，或进程内执行完整采集栈；
/// `plan run` 在内联模式下前台跑一轮（dispatch_round 同步执行，不重置游标）。
async fn cli_plan(rest: Vec<String>) -> Result<(), assembly::Error> {
    let mut args = rest.into_iter();
    let Some(raw) = args.next() else {
        print!("{}", plan_help());
        std::process::exit(2);
    };
    let verb = match raw.as_str() {
        "ls" | "list" => "plan.ls",
        "get" => "plan.get",
        "new" | "create" => "plan.new",
        "set" | "update" => "plan.set",
        "pause" => "plan.pause",
        "resume" => "plan.resume",
        "run" | "continue" => "plan.run",
        other => {
            return Err(assembly::Error::new(format!(
                "未知 plan 动词: {other}（可用: ls / get / new / set / pause / resume / run；别名: list / create / update / continue）"
            )));
        }
    };
    let mut positional: Vec<String> = Vec::new();
    let mut flags: serde_json::Map<String, Value> = serde_json::Map::new();
    let mut data = PathBuf::from("var");
    let mut config = PathBuf::from("config");
    let mut http = env::var("MODALPRISM_HTTP").ok().filter(|v| !v.trim().is_empty());
    let mut project = String::new();
    let mut inline = false;
    let mut from_search: Option<String> = None;
    let mut into: Option<String> = None;
    let mut pretty = true;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{}", plan_help());
                return Ok(());
            }
            "--data" => data = PathBuf::from(take_value(&mut args, &arg)?),
            "--config" => config = PathBuf::from(take_value(&mut args, &arg)?),
            "--http" => http = Some(take_value(&mut args, &arg)?),
            "--project" => project = take_value(&mut args, &arg)?,
            "--inline" => inline = true,
            "--json" => pretty = false,
            "--from-search" => from_search = Some(take_value(&mut args, &arg)?),
            "--into" => into = Some(take_value(&mut args, &arg)?),
            "-n" | "--limit" => {
                flags.insert("limit".into(), json!(take_value(&mut args, &arg)?));
            }
            "--vectorize" | "--deliveryEnabled" | "--enabled" => {
                flags.insert(arg.trim_start_matches("--").to_string(), json!(true));
            }
            other if other.starts_with("--") => {
                let value = take_value(&mut args, &arg)?;
                let name = other.trim_start_matches("--").to_string();
                // 结构类字段按 JSON 解析（--trigger '{"kind":"oneShot"}'），解析失败按字符串交给领域层校验。
                let parsed: Value = serde_json::from_str(&value).unwrap_or(Value::String(value));
                flags.insert(name, parsed);
            }
            other => positional.push(other.to_string()),
        }
    }
    let mut payload = flags;
    match verb {
        "plan.ls" => {}
        "plan.get" | "plan.pause" | "plan.resume" | "plan.run" | "plan.set" => {
            let plan_id = positional
                .first()
                .cloned()
                .or_else(|| {
                    payload
                        .get("planId")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .ok_or_else(|| assembly::Error::new(format!("缺少计划标识: mp plan {verb} <planId>")))?;
            payload.insert("planId".into(), json!(plan_id));
        }
        "plan.new" => {
            let targets: Vec<String> = positional.clone();
            if let Some(id) = &from_search {
                // --from-search：从已保存的检索记录取 URL 目标，生成或追加 oneShot 计划。
                let record = load_search_record(&data, id)?;
                let urls = search_record_urls(&record);
                if urls.is_empty() {
                    return Err(assembly::Error::new(format!(
                        "检索记录 {id} 没有 URL 结果，无法生成计划"
                    )));
                }
                if let Some(plan_id) = &into {
                    // 追加到既有计划：等价 plan.set appendUrls。
                    payload.insert("planId".into(), json!(plan_id));
                    payload.insert("appendUrls".into(), json!(urls));
                    return run_plan_tool("plan.set", payload, &data, &config, &http, &project, pretty, inline).await;
                }
                payload.entry("name".to_string()).or_insert_with(|| {
                    json!(record
                        .pointer("/result/query")
                        .and_then(Value::as_str)
                        .map(|query| format!("检索采集：{query}"))
                        .unwrap_or_else(|| "检索采集计划".to_string()))
                });
                payload.insert("urls".into(), json!(urls));
                payload.entry("trigger".to_string()).or_insert_with(|| json!({"kind": "oneShot", "fireAt": "immediate"}));
            } else {
                if targets.is_empty() {
                    return Err(assembly::Error::new(
                        "mp plan new 至少要有一个目标信源（位置参数）或 --from-search <searchId>",
                    ));
                }
                payload.insert("targets".into(), json!(targets));
            }
        }
        other => unreachable!("已穷举 plan 动词: {other}"),
    }
    run_plan_tool(verb, payload, &data, &config, &http, &project, pretty, inline).await
}

/// 计划动词的统一执行：附加或进程内，单次工具调用（含 plan.run 的前台一轮）。
async fn run_plan_tool(
    verb: &str,
    payload: serde_json::Map<String, Value>,
    data: &std::path::Path,
    config: &std::path::Path,
    http: &Option<String>,
    project: &str,
    pretty: bool,
    inline: bool,
) -> Result<(), assembly::Error> {
    let payload = Value::Object(payload);
    match runtime(http, inline).await {
        Runtime::Attach(base) => {
            let result = http_tool_call(&base, project, verb, payload)
                .await
                .map_err(|failure| fail(failure.body));
            print_value(&result?, pretty);
            Ok(())
        }
        Runtime::Inline => {
            let running = boot_opts(
                data,
                config,
                Binds {
                    http: "127.0.0.1:0".parse().expect("http"),
                    proxy: "127.0.0.1:0".parse().expect("proxy"),
                    push: "127.0.0.1:0".parse().expect("push"),
                },
                BootOpts {
                    inline: true,
                    ..BootOpts::default()
                },
            )
            .await?;
            let state = running.api();
            // plan.run 在内联模式下同步执行一轮（dispatch_round），跑完再关栈。
            let result = domain_api::tools::call(&state, project, verb, payload.clone(), true).await;
            running.shutdown().await?;
            let value = result.map_err(|error| fail(error.body()))?;
            print_value(&value, pretty);
            Ok(())
        }
    }
}

/// 从检索记录提取 URL 结果（保持检索顺序，去重）。
fn search_record_urls(record: &Value) -> Vec<String> {
    let mut urls = Vec::new();
    for hit in record
        .pointer("/result/results")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        if let Some(url) = hit.get("url").and_then(Value::as_str) {
            if !urls.iter().any(|item| item == url) {
                urls.push(url.to_string());
            }
        }
    }
    urls
}

/// `mp plan` 动词帮助。
fn plan_help() -> String {
    String::from(
        "用法: mp plan <动词> [参数…] [--json] [--data <dir>] [--config <dir>] [--http <地址>]\n\
\n\
动词:\n\
  mp plan ls [--status active|paused|disabled|running|exhausted] [-n <条数>]\n\
  mp plan get <planId>            # 计划详情 + 进度 + 每信源游标\n\
  mp plan new [信源…] [--name <名>] [--trigger '<json>'] [--from-search <searchId>] [--into <planId>]\n\
  mp plan set <planId> [--trigger '<json>'] [--enabled] [--pipeline '<json>'] [--vectorize]\n\
  mp plan pause <planId>          # 主动中断：不派发新任务，已落盘数据与游标保留\n\
  mp plan resume <planId>         # 从上一已接受游标继续\n\
  mp plan run <planId>            # 立即跑一轮（别名 continue）；不重置游标\n\
  --inline                         # 不附加到已在跑的 collector，用 --data/--config 另起进程\n\
\n\
别名: list=ls / create=new / update=set / continue=run\n\
\n\
说明:\n\
  - paused 与 enabled 是两个独立字段：enabled=false 是草稿/停用；paused=true 是主动中断。\n\
  - 暂停期间不派发新任务；已开始的任务做完并提交游标后停，不回滚已提交数据。\n\
  - 配置指纹（access/节律/quota 等取数形态）变化会丢弃游标，从新声明的起点重新采集。\n\
  - 游标只对支持跨请求游标的访问器有意义（rss / api / paid）；其余为 null。\n",
    )
}

/// mp strategy 一级子命令：策略动词与 plan 走同一套 CLI 与 MCP 主路径。
///
/// `mp strategy ls|get|new|set|remove`。执行方式与 plan 相同：附加到运行中的
/// collector，或进程内执行完整采集栈；策略是可复用的行为定义，供多个计划引用。
async fn cli_strategy(rest: Vec<String>) -> Result<(), assembly::Error> {
    let mut args = rest.into_iter();
    let Some(raw) = args.next() else {
        print!("{}", strategy_help());
        std::process::exit(2);
    };
    let verb = match raw.as_str() {
        "ls" | "list" => "strategy.ls",
        "get" => "strategy.get",
        "new" | "create" => "strategy.new",
        "set" | "update" => "strategy.set",
        "remove" | "rm" => "strategy.remove",
        other => {
            return Err(assembly::Error::new(format!(
                "未知 strategy 动词: {other}（可用: ls / get / new / set / remove；别名: list / create / update / rm）"
            )));
        }
    };
    let mut positional: Vec<String> = Vec::new();
    let mut flags: serde_json::Map<String, Value> = serde_json::Map::new();
    let mut data = PathBuf::from("var");
    let mut config = PathBuf::from("config");
    let mut http = env::var("MODALPRISM_HTTP").ok().filter(|v| !v.trim().is_empty());
    let mut project = String::new();
    let mut inline = false;
    let mut pretty = true;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{}", strategy_help());
                return Ok(());
            }
            "--data" => data = PathBuf::from(take_value(&mut args, &arg)?),
            "--config" => config = PathBuf::from(take_value(&mut args, &arg)?),
            "--http" => http = Some(take_value(&mut args, &arg)?),
            "--inline" => inline = true,
            "--project" => project = take_value(&mut args, &arg)?,
            "--json" => pretty = false,
            other if other.starts_with("--") => {
                let value = take_value(&mut args, &arg)?;
                let name = other.trim_start_matches("--").to_string();
                // 结构类字段按 JSON 解析（--trigger '{"kind":"recurring","interval":"1h"}'），
                // 解析失败按字符串交给领域层校验。
                let parsed: Value = serde_json::from_str(&value).unwrap_or(Value::String(value));
                flags.insert(name, parsed);
            }
            other => positional.push(other.to_string()),
        }
    }
    let mut payload = flags;
    match verb {
        "strategy.ls" => {}
        "strategy.get" | "strategy.remove" => {
            let strategy_id = positional
                .first()
                .cloned()
                .or_else(|| {
                    payload
                        .get("strategyId")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .ok_or_else(|| {
                    assembly::Error::new(format!("缺少策略标识: mp strategy {verb} <strategyId>"))
                })?;
            payload.insert("strategyId".into(), json!(strategy_id));
        }
        "strategy.new" => {
            // 位置参数是可选的显式 id；未给时由领域层按内容哈希派生。
            if let Some(id) = positional.first() {
                payload.insert("id".into(), json!(id));
            }
        }
        "strategy.set" => {
            let strategy_id = positional
                .first()
                .cloned()
                .or_else(|| {
                    payload
                        .get("strategyId")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .ok_or_else(|| {
                    assembly::Error::new("缺少策略标识: mp strategy set <strategyId>")
                })?;
            payload.insert("strategyId".into(), json!(strategy_id));
        }
        other => unreachable!("已穷举 strategy 动词: {other}"),
    }
    run_plan_tool(verb, payload, &data, &config, &http, &project, pretty, inline).await
}

fn strategy_help() -> String {
    String::from(
        "用法: mp strategy <动词> [参数…] [--json] [--data <dir>] [--config <dir>] [--http <地址>]\n\
\n\
动词:\n\
  mp strategy ls                       # 策略清单：可复用的节律/额度/停止策略\n\
  mp strategy get <strategyId>         # 策略详情\n\
  mp strategy new --trigger '<json>' [--name <名>] [--quota '<json>']   # 新建策略；可给位置参数作 id\n\
  mp strategy set <strategyId> [--trigger '<json>'] [--quota '<json>']  # 修改策略，引用它的计划下次解析生效\n\
  mp strategy remove <strategyId>      # 删除策略；仍被计划引用时拒绝（别名 rm）\n\
\n\
别名: list=ls / create=new / update=set / rm=remove\n\
\n\
说明:\n\
  - 策略是无状态的行为定义；计划通过 strategy 字段引用，各自携带运行状态互不干扰。\n\
  - 计划自身的 trigger/quota 等字段是对所引策略的字段级覆写，未覆写维度回落策略。\n\
  - 策略内容变化即翻版本（strategy/<slug>@<哈希>），run 可追溯生效的策略版本。\n",
    )
}

/// mp 一级短命令：once / search / ls / read / fetch（catalog 是 ls 的别名）。
///
/// 默认进程内执行：起一次完整采集栈（不起采集循环、不做热加载）直接调领域 API 工具面；
/// `--http <地址>` 或 `MODALPRISM_HTTP` 指向时按既有 HTTP 规则附加，未指定时若默认地址上
/// 已有 collector 也附加，避免与运行中的存储双开。
async fn cli_mp(name: &str, rest: Vec<String>) -> Result<(), assembly::Error> {
    let verb = mp_verb(name);
    let spec = surface::tool(verb).expect("mp 子命令必须是工具动词");
    let mut positional: Vec<String> = Vec::new();
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut data = PathBuf::from("var");
    let mut config = PathBuf::from("config");
    let mut http = env::var("MODALPRISM_HTTP").ok().filter(|v| !v.trim().is_empty());
    let mut inline = false;
    let mut project = String::new();
    let mut save = false;
    let mut from_search: Option<String> = None;
    let mut into: Option<String> = None;
    let mut pretty = true;
    let mut args = rest.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{}", verb_help(verb));
                return Ok(());
            }
            "--data" => data = PathBuf::from(take_value(&mut args, &arg)?),
            "--config" => config = PathBuf::from(take_value(&mut args, &arg)?),
            "--http" => http = Some(take_value(&mut args, &arg)?),
            "--inline" => inline = true,
            "--project" => project = take_value(&mut args, &arg)?,
            "--json" => pretty = false,
            "--save" => save = true,
            "--from-search" => from_search = Some(take_value(&mut args, &arg)?),
            "--into" => into = Some(take_value(&mut args, &arg)?),
            "-n" | "--limit" => pairs.push(("limit".into(), take_value(&mut args, &arg)?)),
            other if other.starts_with("--") => {
                let value = take_value(&mut args, &arg)?;
                pairs.push((other.trim_start_matches("--").to_string(), value));
            }
            other => positional.push(other.to_string()),
        }
    }
    if !from_search.is_none() {
        if verb != "once" {
            return Err(assembly::Error::new("--from-search 只用于 mp once"));
        }
        if !positional.is_empty() {
            return Err(assembly::Error::new(
                "mp once --from-search <searchId> 不接受位置参数（URL 来自检索结果）",
            ));
        }
    }
    // `--file` 只在 CLI 入口开放：与进程同主体，走统一摄入函数（摄入根 <data_dir>/inbox）。
    let file_value = pairs
        .iter()
        .position(|(name, _)| name.eq_ignore_ascii_case("file"))
        .map(|index| pairs.remove(index).1);

    let mut payload = build_payload(spec, &positional, &pairs)?;
    // --save 语义：once 落盘（mode=store）；调用者显式给过 mode 时不覆盖。
    if save && verb == "once" && !payload.contains_key("mode") {
        payload.insert("mode".to_string(), json!("store"));
    }
    if let Some(path) = file_value {
        payload.insert("file".to_string(), json!(path));
    }
    for param in spec.params.iter().filter(|param| param.required) {
        if !payload.contains_key(param.name) && !(verb == "once" && from_search.is_some()) {
            return Err(assembly::Error::new(format!(
                "缺少必填参数: {}{}（用法见 mp {verb} --help）",
                param.name,
                spec.params
                    .iter()
                    .find(|item| item.name == param.name)
                    .map(|item| format!("（{}）", item.summary))
                    .unwrap_or_default()
            )));
        }
    }

    let payload = Value::Object(payload);
    match runtime(&http, inline).await {
        Runtime::Attach(base) => {
            let executor = Executor::Http(&base, &project);
            finish(execute(&executor, verb, &payload, &from_search, &into, save, &data).await, pretty)
        }
        Runtime::Inline => {
            let running = boot_opts(
                &data,
                &config,
                Binds {
                    http: "127.0.0.1:0".parse().expect("http"),
                    proxy: "127.0.0.1:0".parse().expect("proxy"),
                    push: "127.0.0.1:0".parse().expect("push"),
                },
                BootOpts {
                    inline: true,
                    ..BootOpts::default()
                },
            )
            .await?;
            let state = running.api();
            let executor = Executor::Inline(&state, &project);
            let result = execute(&executor, verb, &payload, &from_search, &into, save, &data).await;
            running.shutdown().await?;
            finish(result, pretty)
        }
    }
}

enum Runtime {
    Inline,
    Attach(String),
}

/// 附加规则：`--http` / `MODALPRISM_HTTP` 显式指定优先；否则默认地址上有
/// collector 在跑就附加，避免与运行中的存储双开；都没有就进程内执行。
async fn runtime(http: &Option<String>, inline: bool) -> Runtime {
    if inline {
        return Runtime::Inline;
    }
    if let Some(base) = http {
        return Runtime::Attach(base.clone());
    }
    match probe_collector().await {
        Some(base) => Runtime::Attach(base),
        None => Runtime::Inline,
    }
}

/// 工具调用执行器：内联（直接调领域 API）与附加（HTTP 回环）共用一套调用形状。
enum Executor<'a> {
    Inline(&'a domain_api::Shared, &'a str),
    Http(&'a str, &'a str),
}

impl Executor<'_> {
    async fn call(&self, verb: &str, payload: Value) -> Result<Value, Value> {
        match self {
            Executor::Inline(state, project) => domain_api::tools::call(
                state,
                project,
                verb,
                payload,
                // 与进程同主体：CLI 入口允许 --file 走统一摄入函数。
                true,
            )
            .await
            .map_err(|error| error.body()),
            Executor::Http(base, project) => {
                http_tool_call(base, project, verb, payload).await.map_err(|failure| failure.body)
            }
        }
    }
}

async fn execute(
    executor: &Executor<'_>,
    verb: &str,
    payload: &Value,
    from_search: &Option<String>,
    into: &Option<String>,
    save: bool,
    data: &std::path::Path,
) -> Result<CallResult, assembly::Error> {
    if let Some(id) = from_search {
        let record = load_search_record(data, id)?;
        let mode = once_mode(save);
        let mut outcome = BatchOutcome::new(from_search_summary(id, &record, &mode));
        let hits = record
            .pointer("/result/results")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for hit in &hits {
            let Some(url) = hit.get("url").and_then(Value::as_str) else {
                continue;
            };
            let payload = json!({ "url": url, "mode": mode });
            outcome.push(url, executor.call("once", payload).await);
        }
        return Ok(CallResult::Batch(outcome));
    }
    let mut value = executor
        .call(verb, payload.clone())
        .await
        .map_err(fail)?;
    if save && verb == "search" {
        value = save_search_record(data, value)?;
    }
    // `mp search --into <planId>`：把本次检索结果的 URL 追加进既有计划（等价
    // plan.set appendUrls，与 HTTP /collection/plans 共用领域实现）。
    if let Some(plan_id) = into.as_deref().filter(|_| verb == "search") {
        let urls: Vec<String> = value["results"]
            .as_array()
            .map(|hits| {
                hits.iter()
                    .filter_map(|hit| hit["url"].as_str())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if urls.is_empty() {
            return Err(assembly::Error::new(
                "本次检索没有 URL 结果，无法追加进计划",
            ));
        }
        let set_payload = json!({ "planId": plan_id, "appendUrls": urls });
        value = executor.call("plan.set", set_payload).await.map_err(fail)?;
    }
    Ok(CallResult::Single(value))
}

enum CallResult {
    Single(Value),
    Batch(BatchOutcome),
}

fn finish(result: Result<CallResult, assembly::Error>, pretty: bool) -> Result<(), assembly::Error> {
    match result? {
        CallResult::Single(value) => {
            print_value(&value, pretty);
            Ok(())
        }
        CallResult::Batch(outcome) => finish_batch(outcome, pretty),
    }
}

/// 失败可见性：错误 JSON 先打到 stdout，再以非零退出码结束。
fn fail(body: Value) -> assembly::Error {
    print_value(&body, true);
    assembly::Error::new(
        body.pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("工具调用失败")
            .to_string(),
    )
}

/// 位置参数按声明顺序填充（跳过已用 --k v 给出的同名字段），与 `tool <动词>` 同一套规则。
fn build_payload(
    spec: &surface::ToolSpec,
    positional: &[String],
    pairs: &[(String, String)],
) -> Result<serde_json::Map<String, Value>, assembly::Error> {
    let mut payload = serde_json::Map::new();
    for (name, value) in pairs {
        let param = spec
            .params
            .iter()
            .find(|param| param.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| {
                assembly::Error::new(format!(
                    "未知参数: --{name}（{} 接受: {}）",
                    spec.id,
                    spec.params
                        .iter()
                        .map(|param| param.name)
                        .collect::<Vec<_>>()
                        .join(" / ")
                ))
            })?;
        payload.insert(param.name.to_string(), coerce(value, param.kind));
    }
    let candidates: Vec<&surface::ToolParam> = spec
        .params
        .iter()
        .filter(|param| {
            !pairs
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case(param.name))
        })
        .collect();
    if positional.len() > candidates.len() {
        return Err(assembly::Error::new(format!(
            "多余的参数: {}（{} 只接受 {} 个位置参数）",
            positional[candidates.len()],
            spec.id,
            spec.params.len()
        )));
    }
    for (param, value) in candidates.iter().zip(positional) {
        payload.insert(param.name.to_string(), coerce(value, param.kind));
    }
    Ok(payload)
}

fn take_value(
    args: &mut std::vec::IntoIter<String>,
    flag: &str,
) -> Result<String, assembly::Error> {
    args.next()
        .ok_or_else(|| assembly::Error::new(format!("缺少参数值: {flag}")))
}

fn print_value(value: &Value, pretty: bool) {
    match pretty {
        true => println!(
            "{}",
            serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
        ),
        false => println!("{value}"),
    }
}

/// 检索结果落盘记录：`mp search <查询> --save` 产物，`mp once --from-search` 的输入。
/// 读写统一收敛在 domain-api（与 API/MCP 的 persist 同一份实现），避免格式漂移。
fn save_search_record(data: &std::path::Path, mut value: Value) -> Result<Value, assembly::Error> {
    let record = domain_api::search::save_search_record(data, &value)
        .map_err(|error| assembly::Error::new(error))?;
    value["searchId"] = record["searchId"].clone();
    Ok(value)
}

fn load_search_record(
    data: &std::path::Path,
    search_id: &str,
) -> Result<Value, assembly::Error> {
    domain_api::search::load_search_record(data, search_id)
        .map_err(|error| assembly::Error::new(error))
}

/// 批量执行结果：任一失败即整体失败（退出码 1），失败详情在 JSON 里。
struct BatchOutcome {
    body: Value,
    succeeded: usize,
    failed: usize,
}

impl BatchOutcome {
    fn new(body: Value) -> Self {
        Self {
            body,
            succeeded: 0,
            failed: 0,
        }
    }

    fn push(&mut self, url: &str, outcome: Result<Value, Value>) {
        let (ok, key, value) = match outcome {
            Ok(value) => (true, "result", value),
            Err(error) => (false, "error", error),
        };
        if ok {
            self.succeeded += 1;
        } else {
            self.failed += 1;
        }
        if let Some(items) = self.body.pointer_mut("/results").and_then(Value::as_array_mut) {
            let mut item = serde_json::Map::new();
            item.insert("url".into(), json!(url));
            item.insert("ok".into(), json!(ok));
            item.insert(key.into(), value);
            items.push(Value::Object(item));
        }
    }
}

fn finish_batch(outcome: BatchOutcome, pretty: bool) -> Result<(), assembly::Error> {
    print_value(&outcome.body, pretty);
    if outcome.failed > 0 {
        return Err(assembly::Error::new(format!(
            "批量采集完成: {} 条成功, {} 条失败",
            outcome.succeeded, outcome.failed
        )));
    }
    Ok(())
}

fn once_mode(save: bool) -> Value {
    if save {
        json!("store")
    } else {
        json!("preview")
    }
}

fn from_search_summary(id: &str, record: &Value, mode: &Value) -> Value {
    json!({
        "searchId": id,
        "mode": mode,
        "query": record.pointer("/result/query").cloned().unwrap_or(Value::Null),
        "results": [],
    })
}

/// 工具调用失败：JSON 体（与 HTTP 响应同形）+ 消息。
struct ToolFailure {
    body: Value,
}

/// 既有附加规则：POST /api/v1/tools/{verb}，CLI 入口标记（--file 允许）。
async fn http_tool_call(
    base: &str,
    project: &str,
    verb: &str,
    payload: Value,
) -> Result<Value, ToolFailure> {
    let mut request = reqwest::Client::new()
        .post(format!("{base}/api/v1/tools/{verb}"))
        .header("x-modalprism-surface", "cli")
        .json(&payload);
    if !project.is_empty() {
        request = request.header("x-project-id", project);
    }
    let response = request.send().await.map_err(|error| ToolFailure {
        body: json!({
            "error": {
                "code": "unavailable",
                "message": format!("领域 API 不可达（先启动 collector，或用 --http 指定地址）: {error}"),
            }
        }),
    })?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let value: Value =
        serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text }));
    if status.is_success() {
        Ok(value)
    } else {
        Err(ToolFailure { body: value })
    }
}

/// 默认地址上有 collector 在跑就附加，避免与运行中的存储双开。
async fn probe_collector() -> Option<String> {
    const DEFAULT_HTTP: &str = "http://127.0.0.1:17880";
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(500))
        .build()
        .ok()?;
    let response = client
        .get(format!("{DEFAULT_HTTP}/api/v1/tools"))
        .send()
        .await
        .ok()?;
    response
        .status()
        .is_success()
        .then(|| DEFAULT_HTTP.to_string())
}

/// `mp agent`：只读 MCP 表面（stdio），与长二进制 modalprism-agent 同一实现。
async fn cli_agent(rest: Vec<String>) -> Result<(), assembly::Error> {
    let command = modalprism_agent::parse_args(rest)
        .map_err(|error| assembly::Error::new(error.to_string()))?;
    let running = modalprism_agent::boot(&command.data, &command.config)
        .await
        .map_err(|error| assembly::Error::new(error.to_string()))?;
    modalprism_agent::serve(running.gateway())
        .await
        .map_err(|error| assembly::Error::new(error.to_string()))
}

/// `tool <动词>` 子命令的帮助：与动词表同源生成，新增动词不用改这里。
fn help() -> String {
    let mut out = String::from(
        "用法: mp tool <动词> [参数…] [--json] [--http <地址>] [--project <id>]\n\n动词：\n",
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
    out.push_str("\n运维透传: mp api <path> [--method GET|POST|PATCH|DELETE] [--body '<json>']\n");
    out
}

/// mp 顶层 `--help` / `mp help`。
fn mp_help() -> String {
    String::from(
        "mp — ModalPrism 命令行（与 modalprism-collector 同一程序）\n\
\n\
用法:\n\
  mp serve [--data <dir>] [--config <dir>] [--http <addr>] [--proxy <addr>] [--push <addr>]\n\
      启动采集进程（与无子命令、长二进制名等价）。\n\
\n\
  mp once <URL> [--save] [--hint content|discover] [--sourceId <id>] [--json]\n\
      一次完成 取数 → 识别 → 适配 → 写入或返回。默认 preview（不落盘），\n\
      --save 落盘（mode=store）。成功退出码 0；失败打印错误 JSON 并退出 1。\n\
  mp once --from-search <searchId> [--save]\n\
      把 `mp search --save` 保存的检索结果逐条采集；任一条失败则整体退出码 1。\n\
\n\
  mp search <查询> [-n <条数>] [--engine <id>] [--save] [--json]\n\
      检索网页；--save 把结果落到 <data>/searches/<searchId>.json，输出带 searchId。\n\
  mp ls [--kind accessor|adapter|type] [--tag <标签>]\n\
  mp catalog                        # ls 的别名\n\
  mp read <内容ID> [--facet <面>] [--variant <变体>]\n\
  mp fetch <URL> [--maxBytes <n>]\n\
\n\
  mp agent [--data <dir>] [--config <dir>]\n\
      启动只读 MCP 表面（stdio），与长二进制名 modalprism-agent 等价。\n\
\n\
  mp plan ls|get|new|set|pause|resume|run [--json]\n\
      计划动词（别名 list / create / update / continue；详见 mp plan）\n\
  mp strategy ls|get|new|set|remove [--json]\n\
      策略动词：可复用的节律/额度/停止策略，供多个计划引用（详见 mp strategy）\n\
  mp tool <动词> …                  # 全部工具动词的运维透传（HTTP，契约不变）\n\
  mp api <路径> …                   # 领域 API 运维透传\n\
\n\
执行方式: once / search / ls / read / fetch 默认进程内执行完整采集栈；\n\
--http <地址> 或 MODALPRISM_HTTP 指定时按既有规则附加到该 collector；\n\
未指定时若 127.0.0.1:17880 上已有 collector 也附加，避免与运行中的存储双开。\n",
    )
}

/// 单个动词的 `mp <动词> --help`：与动词表同源生成。
fn verb_help(verb: &str) -> String {
    let spec = surface::tool(verb).expect("动词存在");
    let mut out = String::from(&format!("用法: mp {verb}"));
    for param in spec.params {
        if param.required {
            out.push_str(&format!(" <{}>", param.name));
        }
    }
    out.push_str(" [--json] [--data <dir>] [--config <dir>] [--http <地址>] [--project <id>]\n\n");
    out.push_str(&format!("{}\n\n参数：\n", spec.summary));
    for param in spec.params {
        out.push_str(&format!(
            "  --{:<12} {:<7} {}{}\n",
            param.name,
            param.kind,
            if param.required { "必填 " } else { "     " },
            param.summary
        ));
    }
    if verb == "search" {
        out.push_str("  -n / --limit    number        检索条数上限\n  --save                        结果落 <data>/searches/<searchId>.json\n");
    }
    if verb == "once" {
        out.push_str("  --save                        落盘（等价 --mode store；缺省 preview）\n  --from-search <searchId>      采集 `mp search --save` 的检索结果\n");
    }
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

#[cfg(test)]
mod mp_tests {
    use super::*;

    /// `mp search <查询>`：位置参数按声明顺序落到 query。
    #[test]
    fn search_positional_maps_to_query() {
        let spec = surface::tool("search").unwrap();
        let payload = build_payload(spec, &["rust 采集".to_string()], &[]).unwrap();
        assert_eq!(payload["query"], json!("rust 采集"));
    }

    /// `mp search <查询> -n 5 --engine bing`：短旗标与透传参数并存。
    #[test]
    fn search_flags_map_to_params() {
        let spec = surface::tool("search").unwrap();
        let payload = build_payload(
            spec,
            &["rust".to_string()],
            &[("limit".to_string(), "5".to_string()), ("engine".to_string(), "bing".to_string())],
        )
        .unwrap();
        assert_eq!(payload["limit"], json!(5.0));
        assert_eq!(payload["engine"], json!("bing"));
        assert_eq!(payload["query"], json!("rust"));
    }

    /// `mp once <URL> --save` → mode=store；缺省无 mode（进程内按 preview 走）。
    #[test]
    fn once_save_maps_to_store_mode() {
        // cli_mp 的注入规则：--save 且调用者没显式给 mode 时才注入 store。
        let mut payload = build_payload(
            surface::tool("once").unwrap(),
            &["https://example.com/a".to_string()],
            &[],
        )
        .unwrap();
        if !payload.contains_key("mode") {
            payload.insert("mode".to_string(), json!("store"));
        }
        assert_eq!(payload["mode"], json!("store"));
        // 显式 mode 不被覆盖。
        let explicit = build_payload(
            surface::tool("once").unwrap(),
            &["https://example.com/a".to_string()],
            &[("mode".to_string(), "preview".to_string())],
        )
        .unwrap();
        assert_eq!(explicit["mode"], json!("preview"));
        assert_eq!(once_mode(true), json!("store"));
        assert_eq!(once_mode(false), json!("preview"));
        let spec = surface::tool("once").unwrap();
        let payload =
            build_payload(spec, &["https://example.com/a".to_string()], &[]).unwrap();
        assert_eq!(payload["url"], json!("https://example.com/a"));
        assert!(!payload.contains_key("mode"), "缺省不注入 mode");
    }

    /// `mp read <ID>`：位置参数落到 contentId。
    #[test]
    fn read_positional_maps_to_content_id() {
        let spec = surface::tool("read").unwrap();
        let payload = build_payload(spec, &["abc-123".to_string()], &[]).unwrap();
        assert_eq!(payload["contentId"], json!("abc-123"));
    }

    /// 未知参数要报错并列出可接受的名字。
    #[test]
    fn unknown_pair_is_rejected() {
        let spec = surface::tool("ls").unwrap();
        let error = build_payload(spec, &[], &[("nope".to_string(), "1".to_string())])
            .expect_err("未知参数应报错");
        assert!(error.to_string().contains("接受"), "实际: {error}");
    }

    /// `mp catalog` 是 `mp ls` 的别名：一级短命令映射到同一动词。
    #[test]
    fn catalog_aliases_ls() {
        assert_eq!(mp_verb("catalog"), "ls");
        assert_eq!(mp_verb("ls"), "ls");
        assert_eq!(mp_verb("once"), "once");
    }

    /// 检索记录落盘往返：`search --save` 写入的文件 `once --from-search` 能读回。
    #[test]
    fn search_record_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let value = json!({ "engine": "ddg", "query": "rust", "count": 1,
            "results": [ { "url": "https://example.com/a", "title": "A" } ] });
        let saved = save_search_record(dir.path(), value).unwrap();
        let search_id = saved["searchId"].as_str().unwrap().to_string();
        assert!(search_id.starts_with('s'));
        let record = load_search_record(dir.path(), &search_id).unwrap();
        assert_eq!(record.pointer("/result/query"), Some(&json!("rust")));
        let urls: Vec<&str> = record
            .pointer("/result/results")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|hit| hit["url"].as_str())
            .collect();
        assert_eq!(urls, vec!["https://example.com/a"]);
        // 不存在的 searchId 给出可操作的失败信息。
        let error = load_search_record(dir.path(), "s404").expect_err("应失败");
        assert!(
            error.to_string().contains("search persist"),
            "失败信息应指向补救动作: {error}"
        );
    }

    /// mp plan 子命令帮助覆盖全部计划动词与别名、暂停语义与游标说明。
    #[test]
    fn plan_help_lists_all_plan_verbs() {
        let text = plan_help();
        for line in [
            "mp plan ls",
            "mp plan get",
            "mp plan new",
            "mp plan set",
            "mp plan pause",
            "mp plan resume",
            "mp plan run",
            "continue=run",
            "--from-search",
            "paused 与 enabled 是两个独立字段",
            "配置指纹",
        ] {
            assert!(text.contains(line), "plan 帮助缺少 {line}");
        }
    }

    /// 批量结果组装：成功/失败分键，任一失败即整体失败。
    #[test]
    fn batch_outcome_tracks_success_and_failure() {
        let mut outcome = BatchOutcome::new(json!({ "results": [] }));
        outcome.push("https://a", Ok(json!({ "mode": "preview" })));
        outcome.push("https://b", Err(json!({ "error": { "message": "x" } })));
        assert_eq!(outcome.succeeded, 1);
        assert_eq!(outcome.failed, 1);
        let results = outcome.body["results"].as_array().unwrap();
        assert_eq!(results[0]["ok"], json!(true));
        assert!(results[0].get("result").is_some());
        assert_eq!(results[1]["ok"], json!(false));
        assert!(results[1].get("error").is_some());
    }

    /// mp 顶层帮助覆盖全部一级子命令与执行方式说明。
    #[test]
    fn mp_help_lists_all_level_one_verbs() {
        let text = mp_help();
        for line in [
            "mp serve",
            "mp once",
            "mp search",
            "mp ls",
            "mp read",
            "mp fetch",
            "mp agent",
            "mp plan",
            "mp strategy",
            "mp tool",
            "mp api",
            "MODALPRISM_HTTP",
        ] {
            assert!(text.contains(line), "帮助缺少 {line}");
        }
    }
}
