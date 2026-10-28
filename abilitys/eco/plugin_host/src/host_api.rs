//! Mod 出站 `$.ns.method` → `ctx.call`（仅 [`mod_outbound_surfaces`] 声明的面，不做插件名/工具名转义）。

use ability_support::{AbilityCtx, Error, MethodId, Result};
use serde_json::{Value, json};
use tokio::time::{Duration, sleep};

pub async fn dispatch_host_api(
    ctx: &dyn AbilityCtx,
    ns: &str,
    method: &str,
    args: &Value,
) -> Result<Value> {
    let arr = args.as_array().cloned().unwrap_or_default();
    match (ns, method) {
        ("fs", "read") => {
            let path = arr.first().and_then(|v| v.as_str()).unwrap_or("");
            ctx.call(
                "read_file",
                &MethodId::new("run").unwrap(),
                json!({ "path": path }),
            )
            .await
        }
        ("fs", "exists") => {
            let path = arr.first().and_then(|v| v.as_str()).unwrap_or("");
            ctx.call(
                "read_file",
                &MethodId::new("run").unwrap(),
                json!({ "path": path }),
            )
            .await
            .map(|_| json!(true))
            .or_else(|_| Ok(json!(false)))
        }
        ("tools", "execute") => {
            let req = arr.first().cloned().unwrap_or(json!({}));
            ctx.call(
                "dispatch",
                &MethodId::new("run").unwrap(),
                json!({ "calls": [req] }),
            )
            .await
        }
        ("process", "run") => bash_from_argv(ctx, &arr).await,
        ("session", "usage") => session_usage(ctx, &arr).await,
        ("session", "cwd") => {
            let cwd = std::env::var("LEAF_WORKSPACE_ROOT")
                .ok()
                .or_else(|| {
                    std::env::current_dir()
                        .ok()
                        .map(|p| p.to_string_lossy().into_owned())
                })
                .unwrap_or_default();
            Ok(json!({ "cwd": cwd }))
        }
        ("ui", method) => ui_outbound(method, &arr),
        ("agents", "get") => agents_get(ctx).await,
        ("clock", "now") => Ok(json!({
            "now": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        })),
        ("clock", "sleep") => {
            let ms = arr
                .first()
                .and_then(|v| v.as_u64())
                .or_else(|| arr.first().and_then(|v| v.as_f64()).map(|f| f as u64))
                .unwrap_or(0);
            sleep(Duration::from_millis(ms)).await;
            Ok(json!({ "ok": true }))
        }
        ("llm", "complete") => llm_complete(ctx, &arr).await,
        ("session", "get") => {
            let id = arr.first().and_then(|v| v.as_str()).unwrap_or("");
            ctx.call(
                "session",
                &MethodId::new("get").unwrap(),
                json!({ "id": id }),
            )
            .await
        }
        ("session", "append_user") => {
            let req = arr.first().cloned().unwrap_or(json!({}));
            ctx.call("session", &MethodId::new("append_user").unwrap(), req)
                .await
        }
        ("session", "append_assistant") => {
            let req = arr.first().cloned().unwrap_or(json!({}));
            ctx.call("session", &MethodId::new("append_assistant").unwrap(), req)
                .await
        }
        ("command", "register") | ("tools", "register") => Ok(json!({ "ok": true })),
        _ => Ok(host_unsupported(ns, method)),
    }
}

fn host_unsupported(ns: &str, method: &str) -> Value {
    json!({
        "code": "host_unsupported",
        "ns": ns,
        "method": method,
    })
}

fn ui_outbound(method: &str, args: &[Value]) -> Result<Value> {
    let payload = json!({
        "ns": "ui",
        "method": method,
        "args": args,
    });
    crate::user_fanout::emit_user("eco.ui", payload);
    Ok(json!({ "ok": true, "surface": format!("ui.{method}") }))
}

async fn session_usage(ctx: &dyn AbilityCtx, arr: &[Value]) -> Result<Value> {
    let sid = arr.first().and_then(|v| v.as_str());
    let mut input_tokens = 0u64;
    let mut output_tokens = 0u64;
    if let Some(id) = sid {
        if catalog_has(ctx, "session") {
            let out = ctx
                .call(
                    "session",
                    &MethodId::new("get").unwrap(),
                    json!({ "id": id }),
                )
                .await;
            if let Ok(v) = out {
                let items = v
                    .get("items")
                    .and_then(|i| i.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                input_tokens = (items as u64) * 128;
                output_tokens = input_tokens / 4;
            }
        }
    }
    let tokens = input_tokens + output_tokens;
    let window = 200_000u64;
    let percent = if window == 0 {
        0.0
    } else {
        (tokens as f64 / window as f64) * 100.0
    };
    Ok(json!({
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "context": {
            "tokens": tokens,
            "window": window,
            "percent": percent
        }
    }))
}

async fn llm_complete(ctx: &dyn AbilityCtx, arr: &[Value]) -> Result<Value> {
    let messages = arr.first().cloned().unwrap_or(json!([]));
    let options = arr.get(1).and_then(|v| v.as_object()).cloned();
    let mut body = json!({ "messages": messages });
    if let (Some(map), Some(opts)) = (body.as_object_mut(), options) {
        for (key, value) in opts {
            if !value.is_null() {
                map.insert(key, value);
            }
        }
    }
    ctx.call("chat", &MethodId::new("completion").unwrap(), body)
        .await
}

async fn agents_get(_ctx: &dyn AbilityCtx) -> Result<Value> {
    let model = std::env::var("LEAF_CHAT_MODEL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "default".into());
    Ok(json!({
        "main": { "model": model }
    }))
}

fn catalog_has(ctx: &dyn AbilityCtx, id: &str) -> bool {
    ctx.catalog()
        .iter()
        .any(|e| e.instance.as_str() == id || e.ability_id.as_str() == id)
}

/// `process.run(argv)` → `bash`：多参数走 `argv` 直启进程，单参数走 shell `command`。
async fn bash_from_argv(ctx: &dyn AbilityCtx, arr: &[Value]) -> Result<Value> {
    let argv = arr
        .first()
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if argv.is_empty() {
        return ctx
            .call(
                "bash",
                &MethodId::new("run").unwrap(),
                json!({ "command": "echo" }),
            )
            .await;
    }
    if argv.len() == 1 {
        let cmd = argv[0].as_str().unwrap_or("echo");
        return ctx
            .call(
                "bash",
                &MethodId::new("run").unwrap(),
                json!({ "command": cmd }),
            )
            .await;
    }
    let strings: Vec<String> = argv
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    if strings.len() != argv.len() {
        return Err(Error::invalid_args("process.run argv 须为字符串数组"));
    }
    ctx.call(
        "bash",
        &MethodId::new("run").unwrap(),
        json!({ "argv": strings }),
    )
    .await
}
