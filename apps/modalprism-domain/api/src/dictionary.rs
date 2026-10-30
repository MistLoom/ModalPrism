//! 语义字典端点：`GET /dictionary`。
//!
//! 内置条目在 `dictionary` crate（纯数据），`config/dictionary.yaml` 可按 `(domain, code)`
//! 覆盖文案。响应带 ETag，前端启动拉一次即可。

use std::path::Path;

use axum::Router;
use axum::extract::State;
use axum::response::Response;
use axum::routing::get;
use serde_json::Value;

use crate::error::ApiError;
use crate::util;
use crate::{AppState, Shared};

pub fn routes() -> Router<Shared> {
    Router::new().route("/dictionary", get(get_dictionary))
}

async fn get_dictionary(State(state): State<Shared>) -> Result<Response, ApiError> {
    let payload = payload(&state);
    let etag = util::etag(&["dictionary", dictionary::VERSION, &payload.to_string()]);
    let mut response = util::ok(&payload);
    if let Ok(value) = etag.parse() {
        response
            .headers_mut()
            .insert(axum::http::header::ETAG, value);
    }
    Ok(response)
}

/// 合并后的字典文档。覆盖文件缺失或格式不对时只用内置条目，不阻断页面。
pub fn payload(state: &AppState) -> Value {
    dictionary::merge(&overrides(&state.config_dir))
}

/// 给一条记录补 `*Label`：键名 → 字典域。列表接口与工具面共用，
/// 这样 CLI / MCP / HTTP 三面拿到的枚举值都能直接展示；未登记的值不补，由调用方回落原值。
pub fn decorate(row: &mut Value, fields: &[(&str, &str)]) {
    let Some(map) = row.as_object_mut() else {
        return;
    };
    for (field, domain) in fields {
        let Some(code) = map.get(*field).and_then(Value::as_str).map(str::to_string) else {
            continue;
        };
        if let Some(entry) = dictionary::lookup(domain, &code) {
            map.insert(format!("{field}Label"), Value::String(entry.label.into()));
        }
    }
}

/// 读 `config/dictionary.yaml`：支持 `{items: [...]}` 与裸数组两种写法。
fn overrides(config_dir: &Path) -> Vec<dictionary::Override> {
    let path = config_dir.join("dictionary.yaml");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(value) = config::parse_yaml("dictionary.yaml", &text) else {
        tracing::warn!("dictionary.yaml 解析失败，忽略覆盖");
        return Vec::new();
    };
    let list = match value {
        Value::Array(items) => items,
        Value::Object(mut map) => match map.remove("items") {
            Some(Value::Array(items)) => items,
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };
    list.into_iter()
        .filter_map(|item| serde_json::from_value::<dictionary::Override>(item).ok())
        .collect()
}
