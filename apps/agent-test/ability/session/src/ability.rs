//! Ability `session`：会话容器。Hub 为 crate 内实现；事件经 ctx.emit。

use std::sync::Arc;

use ability_support::{
    Ability, AbilityCtx, AbilityManifest, AbilityProvider, AbilitySpec, Error, MethodId, Result,
};
use ai_protocol::{Message, ToolCall};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::generated::manifest;
use crate::hub::SessionHub;
use crate::model::{ConversationId, ItemDraft};
use crate::query::ConversationQuery;
use crate::snapshot::{ImportMode, SessionSnapshot};

pub fn provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::from_manifest(manifest::manifest()).into_factory(|_config| async {
        Ok(Arc::new(SessionAbility {
            manifest: manifest::manifest(),
            hub: SessionHub::memory(),
        }) as Arc<dyn Ability>)
    })
}

struct SessionAbility {
    manifest: AbilityManifest,
    hub: SessionHub,
}

#[derive(Deserialize)]
struct IdArgs {
    id: Option<String>,
    title: Option<String>,
    text: Option<String>,
}

#[derive(Deserialize)]
struct AppendAssistantArgs {
    id: String,
    text: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Deserialize)]
struct AppendToolResultsArgs {
    id: String,
    messages: Vec<Message>,
}

#[derive(Deserialize)]
struct ListArgs {
    limit: Option<u32>,
}

#[derive(Deserialize)]
struct CompactArgs {
    id: String,
    until_seq: u64,
    marker_text: Option<String>,
}

#[derive(Deserialize)]
struct ForkArgs {
    id: String,
    until_seq: Option<u64>,
}

#[derive(Deserialize)]
struct ImportArgs {
    snapshot: SessionSnapshot,
    mode: Option<String>,
}

fn map_err(e: crate::error::SessionError) -> Error {
    Error::failed(e.to_string())
}

fn build_assistant_message(text: Option<String>, tool_calls: Option<Vec<ToolCall>>) -> Message {
    let has_tools = tool_calls.as_ref().is_some_and(|c| !c.is_empty());
    if has_tools {
        let mut msg = Message::assistant_tool_calls(tool_calls.unwrap());
        if let Some(t) = text.filter(|s| !s.is_empty()) {
            msg.content = Some(ai_protocol::MessageContent::Text(t));
        }
        msg
    } else {
        Message::assistant(text.unwrap_or_default())
    }
}

#[async_trait]
impl Ability for SessionAbility {
    fn manifest(&self) -> &AbilityManifest {
        &self.manifest
    }

    async fn invoke(&self, method: &MethodId, ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
        let a: IdArgs = serde_json::from_value(args.clone()).unwrap_or(IdArgs {
            id: None,
            title: None,
            text: None,
        });
        match method.as_str() {
            "create" => {
                let mut b = self.hub.create();
                if let Some(t) = a.title {
                    b = b.title(t);
                }
                let s = b.open().await.map_err(map_err)?;
                let id = s.id().to_string();
                ctx.emit("created", json!({ "id": id })).await?;
                Ok(json!({ "id": id }))
            }
            "get" => {
                let id = a.id.ok_or_else(|| Error::invalid_args("缺少 id"))?;
                let cid: ConversationId = id
                    .parse()
                    .map_err(|e: uuid::Error| Error::invalid_args(e.to_string()))?;
                let conv = self.hub.session(cid).get().await.map_err(map_err)?;
                serde_json::to_value(conv).map_err(|e| Error::failed(e.to_string()))
            }
            "append_user" => {
                let id = a.id.ok_or_else(|| Error::invalid_args("缺少 id"))?;
                let text = a.text.ok_or_else(|| Error::invalid_args("缺少 text"))?;
                let cid: ConversationId = id
                    .parse()
                    .map_err(|e: uuid::Error| Error::invalid_args(e.to_string()))?;
                self.hub
                    .session(cid)
                    .append_user(text)
                    .await
                    .map_err(map_err)?;
                ctx.emit("items_appended", json!({ "id": id, "role": "user" }))
                    .await?;
                Ok(json!({ "ok": true }))
            }
            "append_assistant" => {
                let a: AppendAssistantArgs =
                    serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
                let cid: ConversationId =
                    a.id.parse()
                        .map_err(|e: uuid::Error| Error::invalid_args(e.to_string()))?;
                let msg = build_assistant_message(a.text, a.tool_calls);
                self.hub
                    .session(cid)
                    .append_message(msg)
                    .await
                    .map_err(map_err)?;
                ctx.emit("items_appended", json!({ "id": a.id, "role": "assistant" }))
                    .await?;
                Ok(json!({ "ok": true }))
            }
            "append_tool_results" => {
                let a: AppendToolResultsArgs =
                    serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
                let session_id = a.id.clone();
                let cid: ConversationId =
                    a.id.parse()
                        .map_err(|e: uuid::Error| Error::invalid_args(e.to_string()))?;
                let count = a.messages.len();
                let drafts: Vec<ItemDraft> =
                    a.messages.into_iter().map(ItemDraft::message).collect();
                self.hub
                    .session(cid)
                    .append_many(drafts)
                    .await
                    .map_err(map_err)?;
                for _ in 0..count {
                    ctx.emit(
                        "items_appended",
                        json!({ "id": session_id, "role": "tool" }),
                    )
                    .await?;
                }
                Ok(json!({ "ok": true, "count": count }))
            }
            "protocol_messages" => {
                let id = a.id.ok_or_else(|| Error::invalid_args("缺少 id"))?;
                let cid: ConversationId = id
                    .parse()
                    .map_err(|e: uuid::Error| Error::invalid_args(e.to_string()))?;
                let msgs = self
                    .hub
                    .session(cid)
                    .protocol_messages()
                    .await
                    .map_err(map_err)?;
                serde_json::to_value(msgs).map_err(|e| Error::failed(e.to_string()))
            }
            "list" => {
                let a: ListArgs = serde_json::from_value(args).unwrap_or(ListArgs { limit: None });
                let mut q = ConversationQuery::default();
                q.limit = Some(a.limit.unwrap_or(50));
                let page = self.hub.query(q).await.map_err(map_err)?;
                serde_json::to_value(page.items).map_err(|e| Error::failed(e.to_string()))
            }
            "export" => {
                let id = a.id.ok_or_else(|| Error::invalid_args("缺少 id"))?;
                let cid: ConversationId = id
                    .parse()
                    .map_err(|e: uuid::Error| Error::invalid_args(e.to_string()))?;
                let snap = self.hub.export(&cid).await.map_err(map_err)?;
                serde_json::to_value(snap).map_err(|e| Error::failed(e.to_string()))
            }
            "import" => {
                let a: ImportArgs =
                    serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
                let mode = match a.mode.as_deref() {
                    Some("restore") => ImportMode::Restore,
                    _ => ImportMode::Copy,
                };
                let conv = self.hub.import(a.snapshot, mode).await.map_err(map_err)?;
                let id = conv.id.to_string();
                ctx.emit("created", json!({ "id": id })).await?;
                Ok(json!({ "id": id }))
            }
            "compact" => {
                let a: CompactArgs =
                    serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
                let cid: ConversationId =
                    a.id.parse()
                        .map_err(|e: uuid::Error| Error::invalid_args(e.to_string()))?;
                let marker =
                    ItemDraft::system(a.marker_text.unwrap_or_else(|| "[上下文已压缩]".into()));
                self.hub
                    .session(cid)
                    .compact(a.until_seq, marker)
                    .await
                    .map_err(map_err)?;
                Ok(json!({ "ok": true }))
            }
            "fork" => {
                let a: ForkArgs =
                    serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
                let cid: ConversationId =
                    a.id.parse()
                        .map_err(|e: uuid::Error| Error::invalid_args(e.to_string()))?;
                let child = if let Some(seq) = a.until_seq {
                    self.hub.session(cid).fork_until(seq).await
                } else {
                    self.hub.session(cid).fork().await
                }
                .map_err(map_err)?;
                let id = child.id().to_string();
                ctx.emit("created", json!({ "id": id })).await?;
                Ok(json!({ "id": id }))
            }
            other => Err(Error::not_found(other)),
        }
    }
}
