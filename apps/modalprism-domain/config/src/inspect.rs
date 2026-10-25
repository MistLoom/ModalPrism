//! 有效配置的模块清单。
//!
//! 只读视图，供运维端口与界面读取；模块值已经过合并与校验，不再二次解释。

use serde_json::{Map, Value, json};

use crate::snapshot::Snapshot;

/// 列出快照中的全部模块。`config` 是合并后的模块值，`file` 与 `line` 指向定义处。
pub fn modules(snapshot: &Snapshot) -> Value {
    let items: Vec<Value> = snapshot
        .modules
        .iter()
        .map(|(id, module)| {
            let (file, line) = origin_of(snapshot, id);
            let mut item = Map::new();
            item.insert("id".into(), Value::String(id.clone()));
            item.insert("kind".into(), member(module, "kind"));
            item.insert("name".into(), member(module, "name"));
            item.insert("description".into(), member(module, "description"));
            item.insert(
                "enabled".into(),
                Value::Bool(
                    module
                        .get("enabled")
                        .and_then(Value::as_bool)
                        .unwrap_or(true),
                ),
            );
            item.insert("labels".into(), member(module, "labels"));
            let kind = module.get("kind").and_then(Value::as_str).unwrap_or("");
            item.insert(
                "domain".into(),
                Value::String(domain_of_module(kind).into()),
            );
            item.insert("file".into(), Value::String(file));
            item.insert("line".into(), Value::from(line));
            item.insert("config".into(), module.clone());
            Value::Object(item)
        })
        .collect();
    json!({
        "generation": snapshot.generation,
        "system": snapshot.system,
        "modules": items,
    })
}

fn member(module: &Value, key: &str) -> Value {
    module.get(key).cloned().unwrap_or(Value::Null)
}

/// 模块所属功能域：按处理环节归类，而不是按文件类型。
pub fn domain_of_module(kind: &str) -> &'static str {
    match kind {
        "source" => "collect",
        "accessor" => "transport",
        "adapter" => "parse",
        "type" => "content",
        "system" | "preset" => "policy",
        _ => "other",
    }
}

/// 按配置键路径归入功能域，供设置项分组。命中靠前的优先。
pub fn domain_of_path(path: &str) -> &'static str {
    let key = path.to_ascii_lowercase();
    const BUCKETS: &[(&str, &[&str])] = &[
        (
            "scheduling",
            &[
                "schedule", "interval", "cadence", "tick", "nextrun", "cron", "jitter", "round",
            ],
        ),
        (
            "limits",
            &[
                "limit",
                "max",
                "budget",
                "quota",
                "pages",
                "throttle",
                "ratelimit",
                "concurrency",
                "attempts",
                "timeout",
            ],
        ),
        (
            "retention",
            &[
                "retain",
                "retention",
                "keep",
                "purge",
                "ttl",
                "snapshot_retain",
                "facet_keep",
                "prune",
            ],
        ),
        (
            "egress",
            &[
                "egress",
                "proxy",
                "robots",
                "circuit",
                "private",
                "blocklist",
                "allowlist",
                "outbound",
                "firewall",
            ],
        ),
        (
            "media",
            &[
                "media",
                "deriv",
                "embedding",
                "ocr",
                "blob",
                "vector",
                "image",
                "audio",
                "video",
                "download",
            ],
        ),
        (
            "credentials",
            &[
                "credential",
                "secret",
                "token",
                "apikey",
                "api_key",
                "password",
                "auth",
                "session",
                "cookie",
            ],
        ),
        (
            "processing",
            &[
                "workflow",
                "pipeline",
                "parse",
                "extract",
                "render",
                "transform",
                "dedup",
                "simhash",
            ],
        ),
    ];
    for (domain, needles) in BUCKETS {
        if needles.iter().any(|needle| key.contains(needle)) {
            return domain;
        }
    }
    "general"
}

/// 模块定义处取自该模块最短的溯源路径，通常是 `id` 或 `kind`。
fn origin_of(snapshot: &Snapshot, id: &str) -> (String, u32) {
    snapshot
        .origins
        .iter()
        .filter(|((owner, _), _)| owner == id)
        .min_by_key(|((_, path), _)| path.len())
        .map(|(_, origin)| (origin.file.clone(), origin.line))
        .unwrap_or_default()
}
