//! 发布、投递、渠道与出站治理投影。

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::error::ApiError;
use crate::operations::accepted;
use crate::util::{self, cursor, cursor_offset, page};
use crate::{AppState, Shared, new_id, now};

pub fn routes() -> Router<Shared> {
    Router::new()
        .route(
            "/publications",
            get(list_publications).post(create_publication),
        )
        .route("/publications/{publicationId}", get(get_publication))
        .route("/publication/deliveries", get(list_deliveries))
        .route(
            "/publication/deliveries/{deliveryId}/retry",
            post(retry_delivery),
        )
        .route(
            "/publication/channels",
            get(list_channels).post(create_channel),
        )
        .route(
            "/publication/channels/{channelId}",
            axum::routing::patch(update_channel).delete(delete_channel),
        )
        .route("/publication/notifications/test", post(test_notification))
        .route("/publication/egressStatus", get(egress_status))
        .route("/publication/egressHosts/{host}/reset", post(reset_host))
}

#[derive(Debug, Clone, Deserialize)]
struct ListQuery {
    #[serde(default)]
    limit: Option<String>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

async fn list_docs(
    state: &AppState,
    kind: &str,
    query: ListQuery,
    status: Option<&str>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let filter = format!("{kind}:{}", status.unwrap_or(""));
    let offset = cursor_offset(query.cursor.as_deref(), &filter)?;
    let mut docs: Vec<Value> = state
        .store
        .list_documents(kind)
        .await?
        .into_iter()
        .map(|doc| {
            let mut body = doc.body;
            if kind == "channel" {
                mask_channel(&mut body);
            }
            if let Some(map) = body.as_object_mut() {
                map.insert(
                    "etag".into(),
                    Value::String(format!("{kind}-{}-v{}", doc.id, doc.version)),
                );
            }
            body
        })
        .collect();
    if let Some(status) = status {
        docs.retain(|doc| doc["status"] == status);
    }
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
        Some(cursor(&filter, offset + limit))
    } else {
        None
    };
    Ok(util::ok(&page(items, next)))
}

async fn list_channels(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    list_docs(&state, "channel", query, None).await
}

async fn create_channel(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["name", "kind", "enabled", "settings"])?;
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
    let id = new_id("channel");
    let doc = json!({
        "id": id,
        "name": name,
        "kind": kind,
        "enabled": enabled,
        "settings": settings,
        "createdAt": now(),
        "updatedAt": now(),
        "status": "active",
    });
    state.store.insert_document("channel", &id, &doc).await?;
    crate::observe::append_audit(
        &state,
        "publication.channel.create",
        "succeeded",
        "创建投递渠道",
        json!({"id": id, "kind": kind}),
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
    let doc = state
        .store
        .get_document("channel", &channel_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("渠道 {channel_id}")))?;
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
    util::if_match(&headers, &format!("channel-{channel_id}-v{}", doc.version))?;
    let mut next = doc.body;
    next["name"] = Value::String(name);
    next["kind"] = Value::String(kind);
    next["enabled"] = Value::Bool(enabled);
    next["settings"] = Value::Object(settings);
    next["updatedAt"] = Value::String(now());
    state
        .store
        .update_document("channel", &channel_id, &next, Some(doc.version))
        .await?;
    let mut view = next.clone();
    mask_channel(&mut view);
    let mut response = util::ok(&view);
    if let Ok(value) = format!("channel-{channel_id}-v{}", doc.version + 1).parse() {
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
        .get_document("channel", &channel_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("渠道 {channel_id}")))?;
    util::if_match(&headers, &format!("channel-{channel_id}-v{}", doc.version))?;
    state.store.delete_document("channel", &channel_id).await?;
    crate::observe::append_audit(
        &state,
        "publication.channel.delete",
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

async fn list_publications(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    list_docs(
        &state,
        "publication",
        query.clone(),
        query.status.as_deref().filter(|value| !value.is_empty()),
    )
    .await
}

async fn get_publication(
    State(state): State<Shared>,
    Path(publication_id): Path<String>,
) -> Result<Response, ApiError> {
    let doc = state
        .store
        .get_document("publication", &publication_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("发布 {publication_id}")))?;
    Ok(util::ok(&doc.body))
}

async fn create_publication(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["draftId", "channelIds", "scheduledAt"])?;
    let draft_id = util::non_empty(map, "draftId")?;
    let channel_ids = util::strings(map, "channelIds")?;
    if channel_ids.is_empty() {
        return Err(ApiError::validation(
            "channelIds",
            "required",
            "至少选择一个渠道",
        ));
    }
    let scheduled_at = util::optional_text(map, "scheduledAt")?;
    if let Some(text) = scheduled_at.as_deref() {
        let at = time::OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339)
            .map_err(|_| {
                ApiError::validation("scheduledAt", "format", "不是有效的 RFC3339 时间")
            })?;
        if at <= time::OffsetDateTime::now_utc() {
            return Err(ApiError::validation(
                "scheduledAt",
                "outOfRange",
                "排期必须晚于当前时间",
            ));
        }
    }
    let draft = state
        .store
        .get_document("draft", &draft_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("草稿 {draft_id}")))?;
    let mut channels = Vec::new();
    for id in &channel_ids {
        let channel = state
            .store
            .get_document("channel", id)
            .await?
            .ok_or_else(|| ApiError::not_found(format!("渠道 {id}")))?;
        if !channel.body["enabled"].as_bool().unwrap_or(false) {
            return Err(ApiError::conflict(format!("渠道 {id} 已停用")));
        }
        channels.push(channel.body);
    }
    let publication_id = new_id("pub");
    let task_id = new_id("ctask");
    let scheduled = scheduled_at.is_some();
    let document = json!({
        "id": publication_id,
        "draftId": draft_id,
        "draftVersion": draft.body["latestVersion"],
        "title": draft.body["title"],
        "body": draft.body["body"],
        "channelIds": channel_ids,
        "channels": channels.iter().map(|channel| json!({"id": channel["id"], "name": channel["name"], "kind": channel["kind"]})).collect::<Vec<_>>(),
        "scheduledAt": scheduled_at,
        "status": if scheduled { "scheduled" } else { "queued" },
        "deliveryIds": [],
        "taskId": task_id,
        "createdAt": now(),
        "updatedAt": now(),
    });
    state
        .store
        .insert_document("publication", &publication_id, &document)
        .await?;
    crate::operations::record_task(
        &state,
        json!({
            "id": task_id,
            "kind": "publication",
            "title": format!("发布 {}", document["title"]),
            "status": "queued",
            "publicationId": publication_id,
            "createdAt": now(),
            "updatedAt": now(),
        }),
    )
    .await?;
    let operation = crate::operations::start(
        &state,
        if scheduled {
            "publication.schedule"
        } else {
            "publication.publish"
        },
        vec![
            json!({"kind": "publication", "id": publication_id}),
            json!({"kind": "task", "id": task_id}),
        ],
        json!({
            "kind": if scheduled { "publication.schedule" } else { "publication.publish" },
            "publicationId": publication_id,
            "scheduledAt": scheduled_at,
        }),
    )
    .await?;
    Ok(accepted(&operation))
}

async fn list_deliveries(
    State(state): State<Shared>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    list_docs(
        &state,
        "delivery",
        query.clone(),
        query.status.as_deref().filter(|value| !value.is_empty()),
    )
    .await
}

pub async fn execute_scheduled_publish(
    state: &AppState,
    operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let publication_id = action["publicationId"].as_str().unwrap_or_default();
    let scheduled_at = action["scheduledAt"].as_str().unwrap_or_default();
    let at =
        time::OffsetDateTime::parse(scheduled_at, &time::format_description::well_known::Rfc3339)
            .map_err(|_| ApiError::validation("scheduledAt", "format", "不是有效的 RFC3339 时间"))?;
    loop {
        if crate::operations::check_cancel(state, operation_id)
            .await
            .is_err()
        {
            if let Some(doc) = state
                .store
                .get_document("publication", publication_id)
                .await?
                && doc.body["status"] == "scheduled"
            {
                let mut next = doc.body;
                next["status"] = Value::String("cancelled".into());
                next["updatedAt"] = Value::String(now());
                state
                    .store
                    .update_document("publication", publication_id, &next, Some(doc.version))
                    .await?;
                crate::operations::record_task(
                    state,
                    json!({
                        "id": next["taskId"],
                        "kind": "publication",
                        "title": format!("发布 {}", next["title"].as_str().unwrap_or("")),
                        "status": "cancelled",
                        "publicationId": publication_id,
                        "createdAt": next["createdAt"],
                        "updatedAt": now(),
                    }),
                )
                .await?;
            }
            return Err(ApiError::conflict("操作已取消"));
        }
        let remaining = at - time::OffsetDateTime::now_utc();
        if remaining <= time::Duration::ZERO {
            break;
        }
        let wait = remaining
            .min(time::Duration::seconds(1))
            .whole_seconds()
            .max(1) as u64;
        tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
    }
    execute_publish(state, operation_id, action).await
}

/// 进程重启后把中断中的发布/投递标记为失败，避免界面永久停在“进行中”。
pub async fn recover_interrupted(state: &AppState, action: &Value) {
    if let Some(publication_id) = action["publicationId"].as_str()
        && let Ok(Some(doc)) = state
            .store
            .get_document("publication", publication_id)
            .await
        && matches!(
            doc.body["status"].as_str().unwrap_or_default(),
            "scheduled" | "publishing" | "queued"
        )
    {
        let mut next = doc.body;
        next["status"] = Value::String("failed".into());
        next["lastError"] = Value::String("进程重启，发布已中断".into());
        next["updatedAt"] = Value::String(now());
        if state
            .store
            .update_document("publication", publication_id, &next, Some(doc.version))
            .await
            .is_ok()
        {
            let _ = crate::operations::record_task(
                state,
                json!({
                    "id": next["taskId"],
                    "kind": "publication",
                    "title": format!("发布 {}", next["title"].as_str().unwrap_or("")),
                    "status": "failed",
                    "publicationId": publication_id,
                    "createdAt": next["createdAt"],
                    "updatedAt": now(),
                    "error": "进程重启，发布已中断",
                }),
            )
            .await;
        }
    }
    if let Some(delivery_id) = action["deliveryId"].as_str()
        && let Ok(Some(doc)) = state.store.get_document("delivery", delivery_id).await
        && matches!(
            doc.body["status"].as_str().unwrap_or_default(),
            "queued" | "delivering"
        )
    {
        let mut next = doc.body;
        next["status"] = Value::String("failed".into());
        next["lastError"] = Value::String("进程重启，投递已中断".into());
        next["updatedAt"] = Value::String(now());
        let _ = state
            .store
            .update_document("delivery", delivery_id, &next, Some(doc.version))
            .await;
    }
}

pub async fn execute_publish(
    state: &AppState,
    operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let publication_id = action["publicationId"].as_str().unwrap_or_default();
    let doc = state
        .store
        .get_document("publication", publication_id)
        .await?
        .ok_or_else(|| ApiError::not_found(publication_id))?;
    let mut publication = doc.body.clone();
    publication["status"] = Value::String("publishing".into());
    publication["updatedAt"] = Value::String(now());
    state
        .store
        .update_document(
            "publication",
            publication_id,
            &publication,
            Some(doc.version),
        )
        .await?;
    let channels = publication["channels"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let title = publication["title"]
        .as_str()
        .unwrap_or("ModalPrism")
        .to_string();
    let body = publication["body"].as_str().unwrap_or_default().to_string();
    let retry_limit = crate::settings::value(state, "production.deliveryRetries")
        .await?
        .as_u64()
        .unwrap_or(2)
        .min(10) as usize;
    let mut delivered = 0usize;
    let mut failed = 0usize;
    let mut delivery_ids = Vec::new();
    for channel in channels {
        let channel_id = channel["id"].as_str().unwrap_or_default().to_string();
        let delivery_id = new_id("dlv");
        let delivery = json!({
            "id": delivery_id,
            "publicationId": publication_id,
            "channelId": channel_id,
            "channelName": channel["name"],
            "status": "delivering",
            "attempts": [{"at": now(), "status": "delivering"}],
            "createdAt": now(),
            "updatedAt": now(),
            "retryOf": Value::Null,
        });
        state
            .store
            .insert_document("delivery", &delivery_id, &delivery)
            .await?;
        delivery_ids.push(Value::String(delivery_id.clone()));
        let mut attempts = vec![json!({"at": now(), "status": "delivering"})];
        let mut outcome = deliver_to_channel(
            state,
            &channel_id,
            &title,
            &body,
            publication.get("scheduledAt").and_then(Value::as_str),
        )
        .await;
        for _ in 0..retry_limit {
            let Err(error) = &outcome else {
                break;
            };
            attempts.push(json!({"at": now(), "status": "failed", "error": error.message.clone()}));
            outcome = deliver_to_channel(
                state,
                &channel_id,
                &title,
                &body,
                publication.get("scheduledAt").and_then(Value::as_str),
            )
            .await;
        }
        let delivery_doc = state
            .store
            .get_document("delivery", &delivery_id)
            .await?
            .ok_or_else(|| ApiError::not_found(&delivery_id))?;
        let mut next = delivery_doc.body;
        let mut attempt_count = 0usize;
        for item in &attempts {
            if item["status"] == "failed" {
                attempt_count += 1;
            }
        }
        match outcome {
            Ok(remote) => {
                delivered += 1;
                next["status"] = Value::String("delivered".into());
                next["remoteId"] = remote;
                attempts.push(json!({"at": now(), "status": "delivered"}));
                next["attemptCount"] = json!(attempt_count + 1);
            }
            Err(error) => {
                failed += 1;
                next["status"] = Value::String("failed".into());
                next["lastError"] = Value::String(error.message.clone());
                attempts
                    .push(json!({"at": now(), "status": "failed", "error": error.message.clone()}));
                next["attemptCount"] = json!(attempt_count + 1);
            }
        }
        next["attempts"] = Value::Array(attempts);
        next["updatedAt"] = Value::String(now());
        state
            .store
            .update_document("delivery", &delivery_id, &next, Some(delivery_doc.version))
            .await?;
    }
    let status = if failed == 0 {
        "published"
    } else if delivered == 0 {
        "failed"
    } else {
        "partiallyPublished"
    };
    let publication = state
        .store
        .get_document("publication", publication_id)
        .await?
        .ok_or_else(|| ApiError::not_found(publication_id))?;
    let mut next = publication.body;
    next["status"] = Value::String(status.into());
    next["deliveryIds"] = Value::Array(delivery_ids);
    next["updatedAt"] = Value::String(now());
    state
        .store
        .update_document(
            "publication",
            publication_id,
            &next,
            Some(publication.version),
        )
        .await?;
    crate::operations::record_task(
        state,
        json!({
            "id": next["taskId"],
            "kind": "publication",
            "title": format!("发布 {}", next["title"].as_str().unwrap_or("")),
            "status": if failed == 0 { "succeeded" } else { "failed" },
            "publicationId": publication_id,
            "createdAt": next["createdAt"],
            "updatedAt": now(),
            "error": if failed > 0 { json!(format!("{failed} 个渠道投递失败")) } else { Value::Null },
        }),
    )
    .await?;
    crate::observe::append_event(
        state,
        "publication.publish",
        "发布结束",
        json!({"publicationId": publication_id, "status": status, "delivered": delivered, "failed": failed}),
        crate::observe::Refs {
            operation_id: Some(operation_id.to_string()),
            publication_id: Some(publication_id.to_string()),
            ..crate::observe::Refs::default()
        },
    )
    .await?;
    Ok(json!({
        "partial": failed > 0 && delivered > 0,
        "publicationId": publication_id,
        "status": status,
        "delivered": delivered,
        "failed": failed,
    }))
}

async fn deliver_to_channel(
    state: &AppState,
    channel_id: &str,
    title: &str,
    body: &str,
    _scheduled_at: Option<&str>,
) -> Result<Value, ApiError> {
    let channel = state
        .store
        .get_document("channel", channel_id)
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

async fn retry_delivery(
    State(state): State<Shared>,
    Path(delivery_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["reason"])?;
    let reason = util::optional_text(map, "reason")?;
    let delivery = state
        .store
        .get_document("delivery", &delivery_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("投递 {delivery_id}")))?;
    if delivery.body["status"] != "failed" {
        return Err(ApiError::conflict("仅失败投递允许重试"));
    }
    let retry_id = new_id("dlv");
    let retry = json!({
        "id": retry_id,
        "publicationId": delivery.body["publicationId"],
        "channelId": delivery.body["channelId"],
        "channelName": delivery.body["channelName"],
        "status": "queued",
        "attempts": [],
        "createdAt": now(),
        "updatedAt": now(),
        "retryOf": delivery_id,
        "reason": reason,
    });
    state
        .store
        .insert_document("delivery", &retry_id, &retry)
        .await?;
    let operation = crate::operations::start(
        &state,
        "publication.delivery.retry",
        vec![json!({"kind": "delivery", "id": retry_id})],
        json!({"kind": "delivery.retry", "deliveryId": retry_id}),
    )
    .await?;
    Ok(accepted(&operation))
}

pub async fn execute_delivery_retry(
    state: &AppState,
    operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let delivery_id = action["deliveryId"].as_str().unwrap_or_default();
    let delivery = state
        .store
        .get_document("delivery", delivery_id)
        .await?
        .ok_or_else(|| ApiError::not_found(delivery_id))?;
    let publication = state
        .store
        .get_document(
            "publication",
            delivery.body["publicationId"].as_str().unwrap_or_default(),
        )
        .await?
        .ok_or_else(|| ApiError::not_found("发布记录"))?;
    let title = publication.body["title"]
        .as_str()
        .unwrap_or("ModalPrism")
        .to_string();
    let body = publication.body["body"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let channel_id = delivery.body["channelId"].as_str().unwrap_or_default();
    let outcome = deliver_to_channel(state, channel_id, &title, &body, None).await;
    let mut next = delivery.body.clone();
    match outcome {
        Ok(remote) => {
            next["status"] = Value::String("delivered".into());
            next["remoteId"] = remote;
            next["updatedAt"] = Value::String(now());
            if let Some(attempts) = next["attempts"].as_array_mut() {
                attempts.push(json!({"at": now(), "status": "delivered"}));
            }
        }
        Err(error) => {
            next["status"] = Value::String("failed".into());
            next["lastError"] = Value::String(error.message.clone());
            next["updatedAt"] = Value::String(now());
            if let Some(attempts) = next["attempts"].as_array_mut() {
                attempts.push(json!({"at": now(), "status": "failed", "error": error.message}));
            }
        }
    }
    state
        .store
        .update_document("delivery", delivery_id, &next, Some(delivery.version))
        .await?;
    crate::observe::append_event(
        state,
        "publication.delivery",
        "投递重试结束",
        json!({"deliveryId": delivery_id, "status": next["status"]}),
        crate::observe::Refs {
            operation_id: Some(operation_id.to_string()),
            delivery_id: Some(delivery_id.to_string()),
            ..crate::observe::Refs::default()
        },
    )
    .await?;
    Ok(json!({"partial": false, "deliveryId": delivery_id, "status": next["status"]}))
}

async fn test_notification(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["channelId", "eventKind"])?;
    let channel_id = util::non_empty(map, "channelId")?;
    let event_kind = util::optional_text(map, "eventKind")?;
    state
        .store
        .get_document("channel", &channel_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("渠道 {channel_id}")))?;
    let operation = crate::operations::start(
        &state,
        "publication.notification.test",
        vec![json!({"kind": "channel", "id": channel_id})],
        json!({"kind": "notification.test", "channelId": channel_id, "eventKind": event_kind}),
    )
    .await?;
    Ok(accepted(&operation))
}

pub async fn execute_notification_test(
    state: &AppState,
    operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let channel_id = action["channelId"].as_str().unwrap_or_default();
    let outcome = deliver_to_channel(
        state,
        channel_id,
        "ModalPrism 测试通知",
        "收到说明通知链路可用；本测试不会创建发布记录。",
        None,
    )
    .await?;
    crate::observe::append_audit(
        state,
        "publication.notification.test",
        "succeeded",
        "试发通知",
        json!({"channelId": channel_id}),
        Some(crate::observe::Refs::op(operation_id)),
    )
    .await?;
    Ok(json!({"partial": false, "channelId": channel_id, "status": "sent", "detail": outcome}))
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

/// 渠道列表和详情只回传主机，路径与查询参数中的密钥不离开服务端。
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
        "publication.egressHost.reset",
        vec![json!({"kind": "egressHost", "id": host})],
        json!({"kind": "egressHost.reset", "host": host}),
    )
    .await?;
    Ok(accepted(&operation))
}

/// 主机复位由发布模块执行，保持出站治理只有一个入口。
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
        "publication.egress.reset",
        "succeeded",
        "复位出站主机",
        json!({"host": host, "existed": existed}),
        Some(crate::observe::Refs::op(operation_id)),
    )
    .await?;
    Ok(json!({"partial": false, "host": host, "existed": existed}))
}
