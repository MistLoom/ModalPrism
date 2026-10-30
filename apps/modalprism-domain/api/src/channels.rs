//! 投递渠道与出站治理：webhook / 飞书渠道的维护、连通检测，以及出站主机的状态查询与复位。
//!
//! 渠道是内容投递的三类目标之一（`kind = "channel"`）。投递记录由 `delivery` 模块负责，
//! 这里只管目标本身与出网治理：渠道地址只回传主机，路径与查询参数里的密钥不离开服务端。

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{get, patch, post};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::error::ApiError;
use crate::operations::accepted;
use crate::util::{self, cursor, cursor_offset, page};
use crate::{AppState, Shared, new_id, now};

const CHANNEL_KIND: &str = "channel";

pub fn routes() -> Router<Shared> {
    Router::new()
        .route(
            "/delivery/channels",
            get(list_channels).post(create_channel),
        )
        .route(
            "/delivery/channels/{channelId}",
            patch(update_channel).delete(delete_channel),
        )
        .route("/delivery/channels/{channelId}/test", post(test_channel))
        .route("/observability/egressStatus", get(egress_status))
        .route("/observability/egressHosts/{host}/reset", post(reset_host))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListQuery {
    #[serde(default)]
    limit: Option<String>,
    #[serde(default)]
    cursor: Option<String>,
}

async fn list_channels(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let offset = cursor_offset(query.cursor.as_deref(), CHANNEL_KIND)?;
    let mut docs: Vec<Value> = state
        .store
        .list_documents(CHANNEL_KIND)
        .await?
        .into_iter()
        .map(|doc| {
            let mut body = doc.body;
            mask_channel(&mut body);
            if let Some(map) = body.as_object_mut() {
                map.insert(
                    "etag".into(),
                    Value::String(format!("{CHANNEL_KIND}-{}-v{}", doc.id, doc.version)),
                );
            }
            body
        })
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
        Some(cursor(CHANNEL_KIND, offset + limit))
    } else {
        None
    };
    Ok(util::ok(&page(items, next)))
}

fn channel_fields(map: &Map<String, Value>) -> Result<Value, ApiError> {
    let name = util::non_empty(map, "name")?;
    let kind = util::text(map, "kind")?;
    if !matches!(kind.as_str(), "webhook" | "feishu") {
        return Err(ApiError::validation(
            "kind",
            "enum",
            "只支持 webhook 或 feishu",
        ));
    }
    let enabled = util::flag(map, "enabled")?;
    let settings = util::optional_object(map, "settings")?.unwrap_or_default();
    if settings
        .get("url")
        .and_then(Value::as_str)
        .is_some_and(is_masked)
    {
        return Err(ApiError::validation(
            "settings.url",
            "masked",
            "请填写完整的投递地址",
        ));
    }
    validate_url(&settings)?;
    Ok(json!({
        "name": name,
        "kind": kind,
        "enabled": enabled,
        "settings": settings,
        "status": "active",
    }))
}

async fn create_channel(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["name", "kind", "enabled", "settings"])?;
    let fields = channel_fields(map)?;
    let id = new_id("channel");
    let mut doc = fields;
    doc["id"] = Value::String(id.clone());
    doc["createdAt"] = Value::String(now());
    doc["updatedAt"] = Value::String(now());
    state.store.insert_document(CHANNEL_KIND, &id, &doc).await?;
    crate::observe::append_audit(
        &state,
        "delivery.channel.create",
        "succeeded",
        "新增投递渠道",
        json!({"id": id, "kind": doc["kind"]}),
        None,
    )
    .await?;
    let mut view = doc.clone();
    mask_channel(&mut view);
    Ok(util::created(&view))
}

async fn update_channel(
    State(state): State<Shared>,
    Path(channel_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["name", "kind", "enabled", "settings"])?;
    let doc = state
        .store
        .get_document(CHANNEL_KIND, &channel_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("渠道 {channel_id}")))?;
    let name = util::non_empty(map, "name")?;
    let kind = util::text(map, "kind")?;
    if !matches!(kind.as_str(), "webhook" | "feishu") {
        return Err(ApiError::validation(
            "kind",
            "enum",
            "只支持 webhook 或 feishu",
        ));
    }
    let enabled = util::flag(map, "enabled")?;
    let mut settings = util::optional_object(map, "settings")?.unwrap_or_default();
    // 前端回传的是掩码地址时沿用原值，避免把密钥覆盖成 `***`。
    if settings
        .get("url")
        .and_then(Value::as_str)
        .is_some_and(is_masked)
    {
        if let Some(existing) = doc.body["settings"]["url"].as_str() {
            settings.insert("url".into(), Value::String(existing.to_string()));
        } else {
            settings.remove("url");
        }
    }
    validate_url(&settings)?;
    util::if_match(
        &headers,
        &format!("{CHANNEL_KIND}-{channel_id}-v{}", doc.version),
    )?;
    let mut next = doc.body;
    next["name"] = Value::String(name);
    next["kind"] = Value::String(kind);
    next["enabled"] = Value::Bool(enabled);
    next["settings"] = Value::Object(settings);
    next["updatedAt"] = Value::String(now());
    state
        .store
        .update_document(CHANNEL_KIND, &channel_id, &next, Some(doc.version))
        .await?;
    let mut view = next.clone();
    mask_channel(&mut view);
    let mut response = util::ok(&view);
    if let Ok(value) = format!("{CHANNEL_KIND}-{channel_id}-v{}", doc.version + 1).parse() {
        response
            .headers_mut()
            .insert(axum::http::header::ETAG, value);
    }
    Ok(response)
}

async fn delete_channel(
    State(state): State<Shared>,
    Path(channel_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let doc = state
        .store
        .get_document(CHANNEL_KIND, &channel_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("渠道 {channel_id}")))?;
    util::if_match(
        &headers,
        &format!("{CHANNEL_KIND}-{channel_id}-v{}", doc.version),
    )?;
    state
        .store
        .delete_document(CHANNEL_KIND, &channel_id)
        .await?;
    crate::observe::append_audit(
        &state,
        "delivery.channel.delete",
        "succeeded",
        "删除投递渠道",
        json!({"id": channel_id}),
        None,
    )
    .await?;
    Ok(util::accepted_json(
        &json!({"id": channel_id, "deleted": true}),
    ))
}

/// 渠道连通检测：按渠道设置真实发一条测试消息，结果留在操作记录里。
async fn test_channel(
    State(state): State<Shared>,
    Path(channel_id): Path<String>,
) -> Result<Response, ApiError> {
    state
        .store
        .get_document(CHANNEL_KIND, &channel_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("渠道 {channel_id}")))?;
    let operation = crate::operations::start(
        &state,
        "delivery.channel.test",
        vec![json!({"kind": CHANNEL_KIND, "id": channel_id})],
        json!({"kind": "channel.test", "channelId": channel_id}),
    )
    .await?;
    Ok(accepted(&operation))
}

pub async fn execute_channel_test(
    state: &AppState,
    operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let channel_id = action["channelId"].as_str().unwrap_or_default();
    let outcome = deliver_to_channel(
        state,
        channel_id,
        "ModalPrism 测试通知",
        "收到说明渠道可用；本测试不会创建投递记录。",
        None,
    )
    .await?;
    crate::observe::append_audit(
        state,
        "delivery.channel.test",
        "succeeded",
        "试发渠道消息",
        json!({"channelId": channel_id}),
        Some(crate::observe::Refs::op(operation_id)),
    )
    .await?;
    Ok(json!({"channelId": channel_id, "status": "sent", "detail": outcome}))
}

/// 供内容投递复用的渠道提交：失败语义由调用方记录，不回滚已成功的其他目标。
pub(crate) async fn deliver_to_channel(
    state: &AppState,
    channel_id: &str,
    title: &str,
    body: &str,
    _scheduled_at: Option<&str>,
) -> Result<Value, ApiError> {
    let channel = state
        .store
        .get_document(CHANNEL_KIND, channel_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("渠道 {channel_id}")))?;
    let kind = channel.body["kind"].as_str().unwrap_or("webhook");
    let url = channel.body["settings"]["url"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::validation("settings.url", "required", "渠道缺少投递地址"))?;
    let notify = workflow::Notify {
        kind: if kind == "feishu" {
            workflow::NotifyKind::Feishu
        } else {
            workflow::NotifyKind::Webhook
        },
        url: url.to_string(),
        on: Vec::new(),
        title: Some(title.to_string()),
    };
    let payload = json!({
        "title": title,
        "message": body.chars().take(4000).collect::<String>(),
        "source": "ModalPrism",
    });
    workflow::deliver(&notify, &payload, &state.services.proxy_setting())
        .await
        .map_err(|error| ApiError::dependency(error.to_string()))?;
    Ok(json!({"channelId": channel_id, "kind": kind}))
}

async fn egress_status(State(state): State<Shared>) -> Result<Response, ApiError> {
    let report = state
        .services
        .egress_report()
        .await
        .map_err(ApiError::from)?;
    let mut value = serde_json::to_value(report).unwrap_or_else(|_| json!({}));
    sanitize(&mut value);
    Ok(util::ok(&value))
}

fn validate_url(settings: &Map<String, Value>) -> Result<(), ApiError> {
    let Some(url) = settings.get("url").and_then(Value::as_str) else {
        return Ok(());
    };
    if url.is_empty() || is_masked(url) {
        return Ok(());
    }
    let parsed = url::Url::parse(url)
        .map_err(|_| ApiError::validation("settings.url", "format", "不是有效地址"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(ApiError::validation(
            "settings.url",
            "scheme",
            "只接受 http/https",
        ));
    }
    Ok(())
}

const MASK: &str = "***";

/// 掩码形式为 `scheme://host/***`；用后缀判断，避免把正常查询串误当掩码。
fn is_masked(url: &str) -> bool {
    url == MASK || url.ends_with("/***")
}

/// 渠道列表与详情只回传主机，路径与查询参数中的密钥不离开服务端。
fn mask_channel(body: &mut Value) {
    let Some(url) = body["settings"]["url"].as_str() else {
        return;
    };
    if url.is_empty() {
        return;
    }
    let masked = url::Url::parse(url)
        .map(|parsed| {
            let scheme = parsed.scheme();
            let host = parsed.host_str().unwrap_or_default();
            let port = parsed
                .port()
                .map(|port| format!(":{port}"))
                .unwrap_or_default();
            format!("{scheme}://{host}{port}/{MASK}")
        })
        .unwrap_or_else(|_| MASK.to_string());
    body["settings"]["url"] = Value::String(masked);
}

fn sanitize(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("password");
            map.remove("token");
            map.remove("credential");
            if let Some(Value::String(url)) = map.get_mut("url")
                && let Ok(mut parsed) = url::Url::parse(url)
            {
                let _ = parsed.set_username("");
                let _ = parsed.set_password(None);
                *url = parsed.to_string();
            }
            for child in map.values_mut() {
                sanitize(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                sanitize(item);
            }
        }
        _ => {}
    }
}

async fn reset_host(
    State(state): State<Shared>,
    Path(host): Path<String>,
) -> Result<Response, ApiError> {
    let operation = crate::operations::start(
        &state,
        "egress.host.reset",
        vec![json!({"kind": "egressHost", "id": host})],
        json!({"kind": "egressHost.reset", "host": host}),
    )
    .await?;
    Ok(accepted(&operation))
}

/// 主机复位与状态查询同源，出站治理只有一个入口。
pub async fn execute_egress_reset(
    state: &AppState,
    operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let host = action["host"].as_str().unwrap_or_default();
    let existed = state
        .services
        .egress_reset(host)
        .await
        .map_err(ApiError::from)?;
    crate::observe::append_audit(
        state,
        "egress.host.reset",
        "succeeded",
        "复位出站主机",
        json!({"host": host, "existed": existed}),
        Some(crate::observe::Refs::op(operation_id)),
    )
    .await?;
    Ok(json!({"host": host, "existed": existed}))
}
