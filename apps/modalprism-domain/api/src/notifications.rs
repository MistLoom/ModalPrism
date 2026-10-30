//! 通知管理：本机通知中心 + 外部平台通知推送。
//!
//! 两者共用一套「事件」语义（计划启动、采集完成、内容投递完成等）：
//! - 通知中心：留在本机，工作台可查看最近条目，并按分类开关。
//! - 通知推送：把同一事件发到外部平台（飞书、企业微信、钉钉、Slack、Discord、Telegram 或通用 Webhook）。
//!
//! 每个渠道只保存必要设置与环境变量引用名，不落明文密钥。

use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{get, patch, post};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::error::ApiError;
use crate::util::{self, cursor, cursor_offset, page};
use crate::{AppState, Shared, new_id, now};

const ENTRY_KIND: &str = "notification_entry";
const SETTINGS_KIND: &str = "notification_center_settings";
const SETTINGS_ID: &str = "center";
const PUSH_KIND: &str = "push_channel";
const RECORD_KIND: &str = "push_record";

/// 通知分类：与前端开关一一对应。
pub const CATEGORIES: &[(&str, &str)] = &[
    ("plan", "计划"),
    ("source", "信源"),
    ("content", "内容"),
    ("delivery", "投递"),
    ("system", "系统"),
];

pub const EVENTS: &[(&str, &str)] = &[
    ("plan.started", "计划启动"),
    ("collection.completed", "采集完成"),
    ("content.produced", "内容入库"),
    ("content.delivered", "内容投递完成"),
    ("source.unhealthy", "信源异常"),
    ("system.notice", "系统通知"),
];

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/notifications/center", get(list_center).post(mark_read))
        .route("/notifications/center/settings", get(get_settings).patch(update_settings))
        .route("/notifications/center/catalog", get(center_catalog))
        .route("/notifications/push/platforms", get(list_platforms))
        .route(
            "/notifications/push/channels",
            get(list_channels).post(create_channel),
        )
        .route(
            "/notifications/push/channels/{channelId}",
            patch(update_channel).delete(delete_channel),
        )
        .route("/notifications/push/test", post(test_push))
        .route("/notifications/push/records", get(list_records))
        .route("/notifications/push/records/{recordId}", get(get_record))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListQuery {
    #[serde(default)]
    limit: Option<String>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    channel_id: Option<String>,
}

fn platform_catalog() -> Vec<Value> {
    vec![
        json!({
            "id": "feishu", "name": "飞书群机器人", "icon": "feishu",
            "note": "群自定义机器人 Webhook，正文用文本消息",
            "fields": [{"key": "url", "label": "Webhook 地址", "required": true, "secret": false, "placeholder": "https://open.feishu.cn/open-apis/bot/v2/hook/…"}],
        }),
        json!({
            "id": "wecom", "name": "企业微信群机器人", "icon": "wecom",
            "note": "群机器人 Webhook，按企业微信文本消息格式发送",
            "fields": [{"key": "url", "label": "Webhook 地址", "required": true, "secret": false, "placeholder": "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=…"}],
        }),
        json!({
            "id": "dingtalk", "name": "钉钉群机器人", "icon": "dingtalk",
            "note": "群机器人 Webhook，按钉钉文本消息格式发送",
            "fields": [{"key": "url", "label": "Webhook 地址", "required": true, "secret": false, "placeholder": "https://oapi.dingtalk.com/robot/send?access_token=…"}],
        }),
        json!({
            "id": "slack", "name": "Slack", "icon": "slack",
            "note": "Incoming Webhook，消息体只取 text 字段",
            "fields": [{"key": "url", "label": "Webhook 地址", "required": true, "secret": false, "placeholder": "https://hooks.slack.com/services/…"}],
        }),
        json!({
            "id": "discord", "name": "Discord", "icon": "discord",
            "note": "频道 Webhook，消息体只取 content 字段",
            "fields": [{"key": "url", "label": "Webhook 地址", "required": true, "secret": false, "placeholder": "https://discord.com/api/webhooks/…"}],
        }),
        json!({
            "id": "telegram", "name": "Telegram", "icon": "telegram",
            "note": "Bot Token 只保存环境变量引用名，不落明文",
            "fields": [
                {"key": "tokenRef", "label": "Bot Token 环境变量名", "required": true, "secret": true, "placeholder": "TELEGRAM_BOT_TOKEN"},
                {"key": "chatId", "label": "会话标识", "required": true, "secret": false, "placeholder": "-1001234567890"}
            ],
        }),
        json!({
            "id": "webhook", "name": "通用 Webhook", "icon": "webhook",
            "note": "POST 结构化 JSON（title、message、event、source）",
            "fields": [{"key": "url", "label": "接收地址", "required": true, "secret": false, "placeholder": "https://example.com/hooks/modalprism"}],
        }),
    ]
}

async fn list_platforms() -> Result<Response, ApiError> {
    Ok(util::ok(&json!({
        "items": platform_catalog(),
        "events": EVENTS.iter().map(|(id, label)| json!({"id": id, "label": label})).collect::<Vec<_>>(),
    })))
}

async fn center_catalog() -> Result<Response, ApiError> {
    Ok(util::ok(&json!({
        "categories": CATEGORIES.iter().map(|(id, label)| json!({"id": id, "label": label})).collect::<Vec<_>>(),
    })))
}

fn settings_doc() -> Value {
    json!({
        "id": SETTINGS_ID,
        "enabled": true,
        "desktop": true,
        "categories": {
            "plan": true,
            "source": true,
            "content": true,
            "delivery": true,
            "system": true,
        },
        "updatedAt": now(),
    })
}

async fn read_settings(state: &AppState) -> Value {
    state
        .store
        .get_document(SETTINGS_KIND, SETTINGS_ID)
        .await
        .ok()
        .flatten()
        .map(|doc| doc.body)
        .unwrap_or_else(settings_doc)
}

async fn get_settings(State(state): State<Shared>) -> Result<Response, ApiError> {
    Ok(util::ok(&read_settings(&state).await))
}

async fn update_settings(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["enabled", "desktop", "categories"])?;
    let current = read_settings(&state).await;
    let mut next = current.clone();
    if let Some(enabled) = map.get("enabled") {
        next["enabled"] = enabled.clone();
    }
    if let Some(desktop) = map.get("desktop") {
        next["desktop"] = desktop.clone();
    }
    if let Some(categories) = util::optional_object(map, "categories")? {
        let mut merged = current["categories"].as_object().cloned().unwrap_or_default();
        for (key, value) in categories {
            if !CATEGORIES.iter().any(|(id, _)| *id == key) {
                return Err(ApiError::validation(
                    "categories",
                    "unknown",
                    format!("未知通知分类: {key}"),
                ));
            }
            if !value.is_boolean() {
                return Err(ApiError::validation(
                    "categories",
                    "type",
                    "分类开关必须是布尔值",
                ));
            }
            merged.insert(key, value);
        }
        next["categories"] = Value::Object(merged);
    }
    next["updatedAt"] = Value::String(now());
    match state.store.get_document(SETTINGS_KIND, SETTINGS_ID).await? {
        Some(doc) => {
            state
                .store
                .update_document(SETTINGS_KIND, SETTINGS_ID, &next, Some(doc.version))
                .await?;
        }
        None => {
            state
                .store
                .insert_document(SETTINGS_KIND, SETTINGS_ID, &next)
                .await?;
        }
    }
    Ok(util::ok(&next))
}

async fn list_center(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let category = query.category.clone().filter(|value| !value.is_empty());
    let tag = format!("notification:{}", category.clone().unwrap_or_default());
    let offset = cursor_offset(query.cursor.as_deref(), &tag)?;
    let mut entries: Vec<Value> = state
        .store
        .list_documents(ENTRY_KIND)
        .await?
        .into_iter()
        .map(|doc| doc.body)
        .filter(|body| {
            category
                .as_deref()
                .map(|value| body["category"].as_str() == Some(value))
                .unwrap_or(true)
        })
        .collect();
    entries.sort_by(|left, right| {
        right["createdAt"]
            .as_str()
            .unwrap_or("")
            .cmp(left["createdAt"].as_str().unwrap_or(""))
    });
    let unread = entries
        .iter()
        .filter(|entry| !entry["read"].as_bool().unwrap_or(false))
        .count();
    let total = entries.len();
    let items: Vec<Value> = entries.into_iter().skip(offset).take(limit + 1).collect();
    let more = items.len() > limit;
    let items: Vec<Value> = items.into_iter().take(limit).collect();
    let next = if more || offset + items.len() < total {
        Some(cursor(&tag, offset + limit))
    } else {
        None
    };
    let mut payload = page(items, next);
    payload["unread"] = json!(unread);
    payload["total"] = json!(total);
    Ok(util::ok(&payload))
}

async fn mark_read(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["ids", "all"])?;
    let all = util::optional_flag(map, "all", false)?;
    let ids = util::optional_strings(map, "ids")?.unwrap_or_default();
    let mut updated = 0usize;
    for doc in state.store.list_documents(ENTRY_KIND).await? {
        if !all && !ids.iter().any(|id| id == &doc.id) {
            continue;
        }
        if doc.body["read"].as_bool().unwrap_or(false) {
            continue;
        }
        let mut next = doc.body.clone();
        next["read"] = json!(true);
        state
            .store
            .update_document(ENTRY_KIND, &doc.id, &next, Some(doc.version))
            .await?;
        updated += 1;
    }
    Ok(util::ok(&json!({"updated": updated})))
}

/// 记录一条通知：通知中心留档，并按分类与事件发给已订阅的推送渠道。
/// 推送失败只写推送记录，不影响调用方。
pub async fn emit(
    state: &AppState,
    category: &str,
    level: &str,
    title: impl Into<String>,
    message: impl Into<String>,
) {
    let title = title.into();
    let message = message.into();
    let settings = read_settings(state).await;
    let enabled = settings["enabled"].as_bool().unwrap_or(true)
        && settings["categories"][category].as_bool().unwrap_or(true);
    if enabled {
        let id = new_id("note");
        let entry = json!({
            "id": id,
            "category": category,
            "level": level,
            "title": title,
            "message": message,
            "read": false,
            "createdAt": now(),
        });
        let _ = state
            .store
            .insert_document(ENTRY_KIND, &id, &entry)
            .await;
        let _ = state.store.trim_documents(ENTRY_KIND, 500).await;
    }
    // 推送：仅发给显式订阅了该事件分类的渠道。
    let event = event_of(category);
    let channels = state.store.list_documents(PUSH_KIND).await.unwrap_or_default();
    for doc in channels {
        if !doc.body["enabled"].as_bool().unwrap_or(false) {
            continue;
        }
        let subscribed = doc.body["events"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item.as_str() == Some(event)));
        if !subscribed {
            continue;
        }
        let body = doc.body.clone();
        let owned = (*state).clone();
        let title = title.clone();
        let message = message.clone();
        let channel_id = doc.id.clone();
        tokio::spawn(async move {
            let platform = body["platform"].as_str().unwrap_or("webhook").to_string();
            let settings = body["settings"].as_object().cloned().unwrap_or_default();
            let outcome = send_push(&owned, &platform, &settings, &title, &message, event).await;
            record_push(
                &owned,
                &channel_id,
                body["name"].as_str().unwrap_or("未命名渠道"),
                &platform,
                event,
                &title,
                outcome,
            )
            .await;
        });
    }
}

fn event_of(category: &str) -> &'static str {
    match category {
        "plan" => "plan.started",
        "source" => "source.unhealthy",
        "content" => "content.produced",
        "delivery" => "content.delivered",
        _ => "system.notice",
    }
}

async fn record_push(
    state: &AppState,
    channel_id: &str,
    channel_name: &str,
    platform: &str,
    event: &str,
    title: &str,
    outcome: Result<(), String>,
) {
    let id = new_id("push");
    let record = match outcome {
        Ok(()) => json!({
            "id": id, "channelId": channel_id, "channelName": channel_name,
            "platform": platform, "event": event, "title": title,
            "status": "sent", "error": Value::Null, "createdAt": now(),
        }),
        Err(error) => json!({
            "id": id, "channelId": channel_id, "channelName": channel_name,
            "platform": platform, "event": event, "title": title,
            "status": "failed", "error": error, "createdAt": now(),
        }),
    };
    let _ = state.store.insert_document(RECORD_KIND, &id, &record).await;
    let _ = state.store.trim_documents(RECORD_KIND, 500).await;
}

fn channel_fields(map: &Map<String, Value>) -> Result<Value, ApiError> {
    let name = util::non_empty(map, "name")?;
    let platform = util::non_empty(map, "platform")?;
    if !platform_catalog()
        .iter()
        .any(|item| item["id"].as_str() == Some(platform.as_str()))
    {
        return Err(ApiError::validation(
            "platform",
            "unknown",
            format!("暂不支持的通知平台: {platform}"),
        ));
    }
    let settings = util::optional_object(map, "settings")?.unwrap_or_default();
    for (key, value) in &settings {
        if !value.is_string() {
            return Err(ApiError::validation(
                "settings",
                "type",
                format!("设置项 {key} 必须是字符串"),
            ));
        }
    }
    let events = util::optional_strings(map, "events")?.unwrap_or_default();
    for event in &events {
        if !EVENTS.iter().any(|(id, _)| id == event) {
            return Err(ApiError::validation(
                "events",
                "unknown",
                format!("未知通知事件: {event}"),
            ));
        }
    }
    let enabled = util::optional_flag(map, "enabled", true)?;
    Ok(json!({
        "name": name,
        "platform": platform,
        "settings": settings,
        "events": events,
        "enabled": enabled,
    }))
}

fn channel_view(mut body: Value, id: &str, version: i64) -> Value {
    if let Some(map) = body.as_object_mut() {
        map.insert(
            "etag".into(),
            Value::String(format!("{PUSH_KIND}-{id}-v{version}")),
        );
    }
    body
}

async fn list_channels(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let tag = "push:channels";
    let offset = cursor_offset(query.cursor.as_deref(), tag)?;
    let mut docs: Vec<Value> = state
        .store
        .list_documents(PUSH_KIND)
        .await?
        .into_iter()
        .map(|doc| channel_view(doc.body, &doc.id, doc.version))
        .collect();
    docs.sort_by(|left, right| {
        right["createdAt"]
            .as_str()
            .unwrap_or("")
            .cmp(left["createdAt"].as_str().unwrap_or(""))
    });
    let total = docs.len();
    let items: Vec<Value> = docs.into_iter().skip(offset).take(limit + 1).collect();
    let more = items.len() > limit;
    let items: Vec<Value> = items.into_iter().take(limit).collect();
    let next = if more || offset + items.len() < total {
        Some(cursor(tag, offset + limit))
    } else {
        None
    };
    Ok(util::ok(&page(items, next)))
}

async fn create_channel(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["name", "platform", "settings", "events", "enabled"])?;
    let fields = channel_fields(map)?;
    let id = new_id("pushch");
    let mut doc = fields;
    doc["id"] = Value::String(id.clone());
    doc["createdAt"] = Value::String(now());
    doc["updatedAt"] = Value::String(now());
    state.store.insert_document(PUSH_KIND, &id, &doc).await?;
    Ok(util::created(&channel_view(doc, &id, 1)))
}

async fn update_channel(
    State(state): State<Shared>,
    Path(channel_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["name", "platform", "settings", "events", "enabled"])?;
    let doc = state
        .store
        .get_document(PUSH_KIND, &channel_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("通知渠道 {channel_id}")))?;
    util::if_match(&headers, &format!("{PUSH_KIND}-{channel_id}-v{}", doc.version))?;
    let merged = json!({
        "name": map.get("name").cloned().unwrap_or_else(|| doc.body["name"].clone()),
        "platform": map.get("platform").cloned().unwrap_or_else(|| doc.body["platform"].clone()),
        "settings": map.get("settings").cloned().unwrap_or_else(|| doc.body["settings"].clone()),
        "events": map.get("events").cloned().unwrap_or_else(|| doc.body["events"].clone()),
        "enabled": map.get("enabled").cloned().unwrap_or_else(|| doc.body["enabled"].clone()),
    });
    let fields = channel_fields(util::object(&merged)?)?;
    let mut next = doc.body.clone();
    for key in ["name", "platform", "settings", "events", "enabled"] {
        next[key] = fields[key].clone();
    }
    next["updatedAt"] = Value::String(now());
    state
        .store
        .update_document(PUSH_KIND, &channel_id, &next, Some(doc.version))
        .await?;
    Ok(util::ok(&channel_view(
        next,
        &channel_id,
        doc.version + 1,
    )))
}

async fn delete_channel(
    State(state): State<Shared>,
    Path(channel_id): Path<String>,
) -> Result<Response, ApiError> {
    state.store.delete_document(PUSH_KIND, &channel_id).await?;
    Ok(util::ok(
        &json!({"id": channel_id, "deleted": true}),
    ))
}

async fn list_records(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let channel = query.channel_id.clone().filter(|value| !value.is_empty());
    let status = query.status.clone().filter(|value| !value.is_empty());
    let tag = format!(
        "push:records:{}:{}",
        channel.clone().unwrap_or_default(),
        status.clone().unwrap_or_default()
    );
    let offset = cursor_offset(query.cursor.as_deref(), &tag)?;
    let mut records: Vec<Value> = state
        .store
        .list_documents(RECORD_KIND)
        .await?
        .into_iter()
        .map(|doc| doc.body)
        .filter(|body| {
            channel
                .as_deref()
                .map(|value| body["channelId"].as_str() == Some(value))
                .unwrap_or(true)
                && status
                    .as_deref()
                    .map(|value| body["status"].as_str() == Some(value))
                    .unwrap_or(true)
        })
        .collect();
    records.sort_by(|left, right| {
        right["createdAt"]
            .as_str()
            .unwrap_or("")
            .cmp(left["createdAt"].as_str().unwrap_or(""))
    });
    let total = records.len();
    let items: Vec<Value> = records.into_iter().skip(offset).take(limit + 1).collect();
    let more = items.len() > limit;
    let items: Vec<Value> = items.into_iter().take(limit).collect();
    let next = if more || offset + items.len() < total {
        Some(cursor(&tag, offset + limit))
    } else {
        None
    };
    Ok(util::ok(&page(items, next)))
}

/// 连通检测：对未保存的草稿设置或已保存渠道发一条测试消息，返回真实结果。
async fn test_push(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["channelId", "platform", "settings"])?;
    let (platform, settings, name) = match map.get("channelId").and_then(Value::as_str) {
        Some(id) => {
            let doc = state
                .store
                .get_document(PUSH_KIND, id)
                .await?
                .ok_or_else(|| ApiError::not_found(format!("通知渠道 {id}")))?;
            (
                doc.body["platform"].as_str().unwrap_or("webhook").to_string(),
                doc.body["settings"].as_object().cloned().unwrap_or_default(),
                doc.body["name"].as_str().unwrap_or(id).to_string(),
            )
        }
        None => (
            util::non_empty(map, "platform")?,
            util::optional_object(map, "settings")?.unwrap_or_default(),
            "草稿渠道".to_string(),
        ),
    };
    let started = std::time::Instant::now();
    let outcome = send_push(
        &state,
        &platform,
        &settings,
        "ModalPrism 测试通知",
        "收到说明通知链路可用；本测试不会创建业务记录。",
        "system.notice",
    )
    .await;
    let latency = started.elapsed().as_millis() as u64;
    match outcome {
        Ok(()) => Ok(util::ok(&json!({
            "ok": true,
            "platform": platform,
            "name": name,
            "latencyMs": latency,
            "detail": "测试消息已送达",
        }))),
        Err(error) => Ok(util::ok(&json!({
            "ok": false,
            "platform": platform,
            "name": name,
            "latencyMs": latency,
            "detail": error,
        }))),
    }
}

/// 平台格式化：只保留各平台真正接受的字段，避免「发出去但被平台忽略」。
fn push_url(platform: &str, settings: &Map<String, Value>) -> Result<String, String> {
    match platform {
        "telegram" => {
            let name = settings
                .get("tokenRef")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "Telegram 需要 Bot Token 环境变量名".to_string())?;
            let token = std::env::var(name)
                .map_err(|_| format!("环境变量 {name} 未设置或不可读"))?;
            Ok(format!("https://api.telegram.org/bot{token}/sendMessage"))
        }
        _ => {
            let url = settings
                .get("url")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "缺少接收地址".to_string())?;
            Ok(url.to_string())
        }
    }
}

fn push_body(
    platform: &str,
    settings: &Map<String, Value>,
    title: &str,
    message: &str,
    event: &str,
) -> Result<Value, String> {
    let text = format!("{title}\n{message}");
    Ok(match platform {
        "feishu" => json!({"msg_type": "text", "content": {"text": text}}),
        "wecom" => json!({"msgtype": "text", "text": {"content": text}}),
        "dingtalk" => json!({"msgtype": "text", "text": {"content": text}}),
        "slack" => json!({"text": text}),
        "discord" => json!({"content": text}),
        "telegram" => {
            let chat_id = settings
                .get("chatId")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "Telegram 需要会话标识".to_string())?;
            json!({"chat_id": chat_id, "text": text})
        }
        _ => json!({
            "source": "ModalPrism",
            "event": event,
            "title": title,
            "message": message,
        }),
    })
}

async fn send_push(
    state: &AppState,
    platform: &str,
    settings: &Map<String, Value>,
    title: &str,
    message: &str,
    event: &str,
) -> Result<(), String> {
    let url = push_url(platform, settings)?;
    let body = push_body(platform, settings, title, message, event)?;
    let headers = vec![("content-type".to_string(), "application/json".to_string())];
    let reply = workflow::http_request(
        "POST",
        &url,
        &headers,
        Some(body.to_string()),
        Duration::from_secs(15),
        &state.services.proxy_setting(),
    )
    .await
    .map_err(|error| error.to_string())?;
    let text = reply.ok("通知平台").map_err(|error| error.to_string())?;
    if platform == "feishu"
        && let Ok(value) = serde_json::from_str::<Value>(&text)
        && value.get("code").and_then(Value::as_i64).unwrap_or(0) != 0
    {
        return Err(format!("飞书返回：{text}"));
    }
    if platform == "wecom"
        && let Ok(value) = serde_json::from_str::<Value>(&text)
        && value.get("errcode").and_then(Value::as_i64).unwrap_or(0) != 0
    {
        return Err(format!("企业微信返回：{text}"));
    }
    Ok(())
}

async fn get_record(
    State(state): State<Shared>,
    Path(record_id): Path<String>,
) -> Result<Response, ApiError> {
    let doc = state
        .store
        .get_document(RECORD_KIND, &record_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("推送记录 {record_id}")))?;
    Ok(util::ok(&doc.body))
}
