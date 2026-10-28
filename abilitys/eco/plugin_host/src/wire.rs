//! 将 Leaf 能力事件转发到 Mod 已注册的钩子名（仅在对应 Ability 内调用）。

use std::collections::HashSet;
use std::sync::Arc;

use ability_support::{
    AbilityCtx, BusEvent, EventClass, EventFilter, EventHandler, EventOpts, EventReg, EventSink,
    InstanceId, Result,
};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::bridge::PluginHostBridge;
use crate::coordinator::coordinator;

struct HookSink {
    hook: String,
    map_payload: fn(&Value) -> Value,
}

async fn live_bridge() -> Option<Arc<PluginHostBridge>> {
    coordinator().bridge().await
}

#[async_trait]
impl EventSink for HookSink {
    async fn on_event(&self, event: &BusEvent) {
        let Some(bridge) = live_bridge().await else {
            return;
        };
        let mapped = (self.map_payload)(event.payload());
        let _ = bridge.call_hook(&self.hook, mapped).await;
    }
}

fn map_turn_complete(payload: &Value) -> Value {
    json!({
        "session_id": payload.get("session_id"),
        "turn_id": payload.get("turn_id"),
        "reason": payload.get("reason"),
        "text": payload.get("text"),
    })
}

fn map_session_start(payload: &Value) -> Value {
    json!({
        "session_id": payload.get("id"),
    })
}

fn map_turn_start(payload: &Value) -> Value {
    json!({
        "session_id": payload.get("session_id"),
        "turn_id": payload.get("turn_id"),
    })
}

fn map_tool_batch(payload: &Value) -> Value {
    payload.clone()
}

fn map_tool_result(payload: &Value) -> Value {
    json!({
        "session_id": payload.get("id"),
        "role": payload.get("role"),
        "content_preview": payload.get("content_preview"),
        "tool_call_id": payload.get("tool_call_id"),
    })
}

/// 注册 Mod 关心的能力事件 → 钩子转发（`turn` / `session` 需在装配中启用）。
pub async fn wire_claude_mod_events(
    ctx: &dyn AbilityCtx,
    hook_names: &[String],
) -> Result<Vec<EventReg>> {
    let hooks: HashSet<&str> = hook_names.iter().map(String::as_str).collect();
    let mut regs = Vec::new();

    if hooks.contains("turn.complete") {
        regs.push(wire_named(ctx, "turn", "completed", "turn.complete", map_turn_complete).await?);
    }
    if hooks.contains("turn.start") {
        regs.push(wire_named(ctx, "turn", "started", "turn.start", map_turn_start).await?);
    }
    if hooks.contains("tool.call") {
        regs.push(wire_named(ctx, "turn", "tool_batch", "tool.call", map_tool_batch).await?);
    }
    if hooks.contains("session.start") {
        regs.push(
            wire_named(
                ctx,
                "session",
                "created",
                "session.start",
                map_session_start,
            )
            .await?,
        );
    }
    if hooks.contains("prompt.submit") {
        regs.push(wire_prompt_submit(ctx).await?);
    }
    if hooks.contains("tool.result") {
        regs.push(wire_tool_result_enriched(ctx).await?);
    }
    Ok(regs)
}

fn map_prompt_submit(payload: &Value) -> Value {
    json!({
        "session_id": payload.get("id"),
        "role": payload.get("role"),
        "text_preview": payload.get("text_preview"),
    })
}

async fn wire_prompt_submit(ctx: &dyn AbilityCtx) -> Result<EventReg> {
    let inst =
        InstanceId::new("session").map_err(|e| ability_support::Error::failed(e.to_string()))?;
    let filter = EventFilter {
        class: Some(EventClass::Ability),
        instance: Some(inst),
        name: Some("items_appended".into()),
    };
    let ctx_hold = ctx.retain();
    struct PromptSink {
        ctx: Arc<dyn AbilityCtx>,
    }
    #[async_trait]
    impl EventSink for PromptSink {
        async fn on_event(&self, event: &BusEvent) {
            if event.payload().get("role").and_then(|v| v.as_str()) != Some("user") {
                return;
            }
            let Some(bridge) = live_bridge().await else {
                return;
            };
            let mut payload = map_prompt_submit(event.payload());
            if let Some(sid) = event.payload().get("id").and_then(|v| v.as_str()) {
                if let Ok(detail) = self
                    .ctx
                    .call(
                        "session",
                        &ability_support::MethodId::new("get").unwrap(),
                        json!({ "id": sid }),
                    )
                    .await
                {
                    let preview = last_user_text(&detail);
                    if let Some(obj) = payload.as_object_mut() {
                        obj.insert("text_preview".into(), json!(preview));
                    }
                }
            }
            let _ = bridge.call_hook("prompt.submit", payload).await;
        }
    }
    ctx.handle(
        filter,
        EventHandler::Message(Arc::new(PromptSink { ctx: ctx_hold })),
        EventOpts::default(),
    )
    .await
}

fn last_user_text(session_get: &Value) -> Option<String> {
    let items = session_get.get("items").and_then(|v| v.as_array())?;
    for item in items.iter().rev() {
        if item.get("role").and_then(|v| v.as_str()) == Some("user") {
            return item
                .get("content")
                .and_then(|c| c.as_str())
                .map(|s| s.chars().take(500).collect());
        }
    }
    None
}

async fn wire_tool_result_enriched(ctx: &dyn AbilityCtx) -> Result<EventReg> {
    let inst =
        InstanceId::new("session").map_err(|e| ability_support::Error::failed(e.to_string()))?;
    let filter = EventFilter {
        class: Some(EventClass::Ability),
        instance: Some(inst),
        name: Some("items_appended".into()),
    };
    let ctx_hold = ctx.retain();
    struct ToolSink {
        ctx: Arc<dyn AbilityCtx>,
    }
    #[async_trait]
    impl EventSink for ToolSink {
        async fn on_event(&self, event: &BusEvent) {
            if event.payload().get("role").and_then(|v| v.as_str()) != Some("tool") {
                return;
            }
            let Some(bridge) = live_bridge().await else {
                return;
            };
            let sid = event.payload().get("id").and_then(|v| v.as_str());
            let mut payload = map_tool_result(event.payload());
            if let Some(id) = sid {
                if let Ok(detail) = self
                    .ctx
                    .call(
                        "session",
                        &ability_support::MethodId::new("get").unwrap(),
                        json!({ "id": id }),
                    )
                    .await
                {
                    if let Some(last) = last_tool_message(&detail) {
                        if let Some(obj) = payload.as_object_mut() {
                            obj.insert("content_preview".into(), json!(last.preview));
                            obj.insert("tool_call_id".into(), json!(last.tool_call_id));
                        }
                    }
                }
            }
            let _ = bridge.call_hook("tool.result", payload).await;
        }
    }
    ctx.handle(
        filter,
        EventHandler::Message(Arc::new(ToolSink { ctx: ctx_hold })),
        EventOpts::default(),
    )
    .await
}

struct ToolMsgPreview {
    preview: String,
    tool_call_id: Option<String>,
}

fn last_tool_message(session_get: &Value) -> Option<ToolMsgPreview> {
    let items = session_get.get("items").and_then(|v| v.as_array())?;
    for item in items.iter().rev() {
        if item.get("role").and_then(|v| v.as_str()) == Some("tool") {
            let content = item.get("content").and_then(|c| c.as_str()).unwrap_or("");
            let preview: String = content.chars().take(800).collect();
            let tool_call_id = item
                .get("tool_call_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            return Some(ToolMsgPreview {
                preview,
                tool_call_id,
            });
        }
    }
    None
}

/// `turn.completed` → Mod `turn.complete`（若 Mod 注册了该钩子）。
pub async fn wire_turn_complete(
    ctx: &dyn AbilityCtx,
    hook_names: &[String],
) -> Result<Vec<EventReg>> {
    wire_claude_mod_events(ctx, hook_names).await
}

async fn wire_named(
    ctx: &dyn AbilityCtx,
    ability: &str,
    event: &str,
    hook: &str,
    map_payload: fn(&Value) -> Value,
) -> Result<EventReg> {
    let inst =
        InstanceId::new(ability).map_err(|e| ability_support::Error::failed(e.to_string()))?;
    let filter = EventFilter {
        class: Some(EventClass::Ability),
        instance: Some(inst),
        name: Some(event.into()),
    };
    let sink = HookSink {
        hook: hook.to_string(),
        map_payload,
    };
    if hook == "tool.result" {
        struct ToolResultSink {
            inner: HookSink,
        }
        #[async_trait]
        impl EventSink for ToolResultSink {
            async fn on_event(&self, event: &BusEvent) {
                if event.payload().get("role").and_then(|v| v.as_str()) != Some("tool") {
                    return;
                }
                let Some(bridge) = live_bridge().await else {
                    return;
                };
                let mapped = (self.inner.map_payload)(event.payload());
                let _ = bridge.call_hook(&self.inner.hook, mapped).await;
            }
        }
        ctx.handle(
            filter,
            EventHandler::Message(Arc::new(ToolResultSink { inner: sink })),
            EventOpts::default(),
        )
        .await
    } else {
        ctx.handle(
            filter,
            EventHandler::Message(Arc::new(sink)),
            EventOpts::default(),
        )
        .await
    }
}
