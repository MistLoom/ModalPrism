//! 请求整形、ETag、游标与字段校验的公共工具。

use axum::http::{HeaderMap, StatusCode};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::error::ApiError;

pub const DEFAULT_LIMIT: usize = 50;
pub const MAX_LIMIT: usize = 200;

pub fn limit(raw: Option<&str>) -> Result<usize, ApiError> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(DEFAULT_LIMIT);
    };
    let value = raw
        .parse::<usize>()
        .map_err(|_| ApiError::validation("limit", "invalid", "必须是正整数"))?;
    if value == 0 || value > MAX_LIMIT {
        return Err(ApiError::validation(
            "limit",
            "outOfRange",
            format!("必须在 1 到 {MAX_LIMIT} 之间"),
        ));
    }
    Ok(value)
}

pub fn page(items: Vec<Value>, next: Option<String>) -> Value {
    json!({"items": items, "nextCursor": next})
}

/// 带总条数的分页响应：总数可知的列表用它，前端据此做页码跳转。
pub fn page_total(items: Vec<Value>, next: Option<String>, total: usize) -> Value {
    json!({"items": items, "nextCursor": next, "total": total})
}

pub fn hash_short(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn fingerprint(method: &str, uri: &str, body: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(method.as_bytes());
    hasher.update(b"\n");
    hasher.update(uri.as_bytes());
    hasher.update(b"\n");
    hasher.update(body);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn cursor(filter: &str, offset: usize) -> String {
    format!("{}:{offset}", hash_short(filter))
}

pub fn cursor_offset(raw: Option<&str>, filter: &str) -> Result<usize, ApiError> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(0);
    };
    let Some((hash, offset)) = raw.split_once(':') else {
        return Err(ApiError::cursor_expired());
    };
    if hash != hash_short(filter) {
        return Err(ApiError::cursor_expired());
    }
    offset
        .parse::<usize>()
        .map_err(|_| ApiError::cursor_expired())
}

/// 页码式分页的偏移解析：`offset` 优先于 `cursor`，两者语义一致（跳过筛选快照前 N 行）。
pub fn page_offset(
    offset: Option<&str>,
    cursor: Option<&str>,
    filter: &str,
) -> Result<usize, ApiError> {
    if let Some(raw) = offset.map(str::trim).filter(|value| !value.is_empty()) {
        return raw
            .parse::<usize>()
            .map_err(|_| ApiError::validation("offset", "invalid", "必须是非负整数"));
    }
    cursor_offset(cursor, filter)
}

/// 资源版本标签：读时返回，写入要求 If-Match 一致。
pub fn etag(parts: &[&str]) -> String {
    let joined = parts.join("/");
    format!("\"{}\"", hash_short(&joined))
}

pub fn if_match(headers: &HeaderMap, expected: &str) -> Result<(), ApiError> {
    let value = headers
        .get("if-match")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::precondition("缺少 If-Match"))?;
    let value = value.trim_matches('"');
    let expected = expected.trim_matches('"');
    if value != expected && value != "*" {
        return Err(ApiError::precondition("资源已被其他会话修改"));
    }
    Ok(())
}

pub fn object(body: &Value) -> Result<&Map<String, Value>, ApiError> {
    body.as_object()
        .ok_or_else(|| ApiError::invalid("请求体必须是 JSON 对象"))
}

pub fn unknown(body: &Map<String, Value>, allowed: &[&str]) -> Result<(), ApiError> {
    for key in body.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(ApiError::unknown_field(key));
        }
    }
    Ok(())
}

pub fn text(body: &Map<String, Value>, key: &str) -> Result<String, ApiError> {
    match body.get(key) {
        Some(Value::String(value)) => Ok(value.clone()),
        Some(Value::Null) | None => Err(ApiError::validation(key, "required", "不能为空")),
        _ => Err(ApiError::validation(key, "type", "必须是字符串")),
    }
}

pub fn non_empty(body: &Map<String, Value>, key: &str) -> Result<String, ApiError> {
    let value = text(body, key)?;
    if value.trim().is_empty() {
        return Err(ApiError::validation(key, "required", "不能为空"));
    }
    Ok(value)
}

pub fn optional_text(body: &Map<String, Value>, key: &str) -> Result<Option<String>, ApiError> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        _ => Err(ApiError::validation(key, "type", "必须是字符串")),
    }
}

pub fn flag(body: &Map<String, Value>, key: &str) -> Result<bool, ApiError> {
    match body.get(key) {
        Some(Value::Bool(value)) => Ok(*value),
        Some(Value::Null) | None => Err(ApiError::validation(key, "required", "不能为空")),
        _ => Err(ApiError::validation(key, "type", "必须是布尔值")),
    }
}

/// 可选布尔：缺省返回 `default`。
pub fn optional_flag(
    body: &Map<String, Value>,
    key: &str,
    default: bool,
) -> Result<bool, ApiError> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        _ => Err(ApiError::validation(key, "type", "必须是布尔值")),
    }
}

pub fn integer(body: &Map<String, Value>, key: &str) -> Result<i64, ApiError> {
    match body.get(key) {
        Some(Value::Number(value)) => value
            .as_i64()
            .ok_or_else(|| ApiError::validation(key, "type", "必须是整数")),
        Some(Value::Null) | None => Err(ApiError::validation(key, "required", "不能为空")),
        _ => Err(ApiError::validation(key, "type", "必须是整数")),
    }
}

pub fn strings(body: &Map<String, Value>, key: &str) -> Result<Vec<String>, ApiError> {
    match body.get(key) {
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let Some(value) = item.as_str() else {
                    return Err(ApiError::validation(key, "type", "数组元素必须是字符串"));
                };
                out.push(value.to_string());
            }
            Ok(out)
        }
        Some(Value::Null) | None => Err(ApiError::validation(key, "required", "不能为空")),
        _ => Err(ApiError::validation(key, "type", "必须是数组")),
    }
}

pub fn optional_strings(
    body: &Map<String, Value>,
    key: &str,
) -> Result<Option<Vec<String>>, ApiError> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(_)) => strings(body, key).map(Some),
        _ => Err(ApiError::validation(key, "type", "必须是数组")),
    }
}

pub fn optional_object(
    body: &Map<String, Value>,
    key: &str,
) -> Result<Option<Map<String, Value>>, ApiError> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(value)) => Ok(Some(value.clone())),
        _ => Err(ApiError::validation(key, "type", "必须是对象")),
    }
}

pub fn uri(body: &Map<String, Value>, key: &str) -> Result<url::Url, ApiError> {
    let value = non_empty(body, key)?;
    let url = url::Url::parse(&value)
        .map_err(|error| ApiError::validation(key, "format", format!("不是有效地址: {error}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ApiError::validation(key, "scheme", "只接受 http/https"));
    }
    Ok(url)
}

pub fn created(value: &Value) -> axum::response::Response {
    use axum::response::IntoResponse;
    (
        StatusCode::CREATED,
        [("content-type", "application/json")],
        value.to_string(),
    )
        .into_response()
}

pub fn ok(value: &Value) -> axum::response::Response {
    use axum::response::IntoResponse;
    (
        StatusCode::OK,
        [("content-type", "application/json")],
        value.to_string(),
    )
        .into_response()
}

pub fn accepted_json(value: &Value) -> axum::response::Response {
    use axum::response::IntoResponse;
    (
        StatusCode::ACCEPTED,
        [("content-type", "application/json")],
        value.to_string(),
    )
        .into_response()
}

/// 由毫秒生成调度声明可读的间隔文本。
pub fn duration_text(ms: i64) -> String {
    if ms <= 0 {
        return "2h".into();
    }
    let seconds = (ms as u64).div_ceil(1000);
    if seconds % 3600 == 0 {
        format!("{}h", seconds / 3600)
    } else if seconds % 60 == 0 {
        format!("{}m", seconds / 60)
    } else {
        format!("{seconds}s")
    }
}

/// 把调度文本换算成毫秒；解析失败时返回 None。
pub fn duration_ms(text: &str) -> Option<i64> {
    let duration = config::duration(text)?;
    Some(duration.as_millis() as i64)
}

/// 由名称生成稳定 slug；非 ASCII 名称回退到时间戳。
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        format!("source-{}", ulid::Ulid::new().to_string().to_lowercase())
    } else {
        out
    }
}
