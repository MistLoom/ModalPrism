//! 方法逻辑：batch
//!
//! 批量操作逐条返回结果，单条失败不中断其余条目：界面需要一次点下去就拿到
//! 「哪几条没成功、为什么」，而不是只看到一个被第一条错误挡住的提示。

use std::sync::Arc;

use serde_json::{Value, json};

pub async fn run(
    ctx: Arc<dyn types::ReadCtx>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let kind = crate::text(&args, "kind")?;
    let action = crate::text(&args, "action")?;
    if !matches!(action.as_str(), "enable" | "disable" | "remove") {
        return Err(ability_support::Error::invalid_args(
            "action 只能是 enable、disable 或 remove",
        ));
    }
    let slugs = args
        .get("slugs")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .ok_or_else(|| ability_support::Error::invalid_args("缺少 slugs"))?;
    let dir = ctx.config_dir();
    let options = ctx.load_options();
    let mut rows = Vec::with_capacity(slugs.len());
    let mut failed = 0usize;
    for slug in slugs {
        let outcome = match action.as_str() {
            "remove" => config::remove(dir, options, &kind, slug),
            _ => config::toggle(dir, options, &kind, slug, action == "enable"),
        };
        match outcome {
            Ok(saved) => rows.push(json!({"slug": slug, "ok": true, "file": saved.file})),
            Err(error) => {
                failed += 1;
                rows.push(json!({"slug": slug, "ok": false, "error": error.to_string()}));
            }
        }
    }
    Ok(json!({
        "kind": kind,
        "action": action,
        "total": rows.len(),
        "failed": failed,
        "results": rows,
    }))
}
