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
