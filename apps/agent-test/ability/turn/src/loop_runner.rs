//! 回合循环核心逻辑。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ability_support::{AbilityCtx, Error, MethodId, Result};
use ai_protocol::{ChatCompletionResponse, Message, ToolCall, repair_tool_call_chain};
use serde_json::{Value, json};

fn m(id: &str) -> MethodId {
    MethodId::new(id).expect("method id")
}

pub struct TurnConfig {
    pub max_rounds: u32,
    pub system_prompt: Option<String>,
}

pub struct RunInput {
    pub session_id: Option<String>,
    pub text: String,
    pub create_session: bool,
    pub skill_paths: Vec<String>,
    pub skill_root: Option<String>,
}

pub struct RunOutput {
    pub session_id: String,
    pub turn_id: String,
    pub reason: &'static str,
    pub text: String,
    pub rounds: u32,
}

pub async fn run_turn(
    ctx: &dyn AbilityCtx,
    config: &TurnConfig,
    cancel: Arc<AtomicBool>,
    turn_id: &str,
    input: RunInput,
) -> Result<RunOutput> {
    let turn_id = turn_id.to_string();

    let session_id = resolve_session_id(ctx, &input).await?;

    ctx.call(
        "session",
        &m("append_user"),
        json!({ "id": session_id, "text": input.text }),
    )
    .await?;

    ctx.emit(
        "started",
        json!({ "session_id": session_id, "turn_id": turn_id }),
    )
    .await?;

    let system = build_system(ctx, config, &input).await?;

    let mut round = 0u32;
    let mut final_text = String::new();
    let mut reason = "stopped";

    while round < config.max_rounds {
        if cancel.load(Ordering::Relaxed) {
            reason = "cancelled";
            break;
        }
        round += 1;

        let mut messages = fetch_messages(ctx, &session_id).await?;
        if let Some(ref sys) = system {
            if !messages.iter().any(|m| m.role == ai_protocol::Role::System) {
                messages.insert(0, Message::system(sys.clone()));
            }
        }

        let tools = ctx.call("dispatch", &m("defs"), json!({})).await?;
        let tool_defs = tools.get("tools").cloned();

        let chat_resp = ctx
            .call(
                "chat",
                &m("completion"),
                json!({
                    "messages": messages,
                    "tools": tool_defs,
                }),
            )
            .await;

        let chat_resp: ChatCompletionResponse = match chat_resp {
            Ok(v) => serde_json::from_value(v).map_err(|e| Error::failed(e.to_string()))?,
            Err(e) => {
                reason = "provider_error";
                ctx.emit(
                    "completed",
                    json!({
                        "session_id": session_id,
                        "turn_id": turn_id,
                        "reason": reason,
                        "text": "",
                    }),
                )
                .await?;
                return Err(e);
            }
        };

        let assistant = chat_resp
            .first_message()
            .ok_or_else(|| Error::failed("模型未返回消息"))?;

        let tool_calls = assistant.tool_calls.clone();
        let content = assistant.content_text().unwrap_or_default();

        if tool_calls.as_ref().is_none_or(|c| c.is_empty()) {
            ctx.call(
                "session",
                &m("append_assistant"),
                json!({ "id": session_id, "text": content }),
            )
            .await?;
            final_text = content;
            ctx.emit(
                "round",
                json!({ "turn_id": turn_id, "round": round, "stop": true }),
            )
            .await?;
            break;
        }

        let calls = tool_calls.unwrap();
        ctx.call(
            "session",
            &m("append_assistant"),
            json!({
                "id": session_id,
                "text": if content.is_empty() { Value::Null } else { json!(content) },
                "tool_calls": calls,
            }),
        )
        .await?;

        ctx.emit(
            "tool_batch",
            json!({ "turn_id": turn_id, "calls": calls_for_event(&calls) }),
        )
        .await?;

        let run_out = match ctx
            .call(
                "dispatch",
                &m("run"),
                json!({ "calls": calls, "continue_on_error": true }),
            )
            .await
        {
            Ok(v) => v,
            Err(e) => {
                let messages: Vec<Message> = calls
                    .iter()
                    .map(|c| {
                        Message::tool_result(
                            c.id.clone(),
                            serde_json::json!({ "error": e.to_string() }).to_string(),
                        )
                    })
                    .collect();
                json!({ "messages": messages })
            }
        };

        ctx.call(
            "session",
            &m("append_tool_results"),
            json!({
                "id": session_id,
                "messages": run_out.get("messages").cloned().unwrap_or(json!([])),
            }),
        )
        .await?;

        ctx.emit(
            "round",
            json!({ "turn_id": turn_id, "round": round, "stop": false }),
        )
        .await?;
    }

    if round >= config.max_rounds && reason == "stopped" {
        reason = "max_rounds";
    }

    ctx.emit(
        "completed",
        json!({
            "session_id": session_id,
            "turn_id": turn_id,
            "reason": reason,
            "text": final_text,
        }),
    )
    .await?;

    Ok(RunOutput {
        session_id,
        turn_id,
        reason,
        text: final_text,
        rounds: round,
    })
}

async fn resolve_session_id(ctx: &dyn AbilityCtx, input: &RunInput) -> Result<String> {
    if let Some(id) = &input.session_id {
        return Ok(id.clone());
    }
    if input.create_session {
        let out = ctx.call("session", &m("create"), json!({})).await?;
        return out
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| Error::failed("session.create 未返回 id"));
    }
    Err(Error::invalid_args(
        "缺少 session_id，或设 create_session=true",
    ))
}

async fn build_system(
    ctx: &dyn AbilityCtx,
    config: &TurnConfig,
    input: &RunInput,
) -> Result<Option<String>> {
    if input.skill_paths.is_empty() && input.skill_root.is_none() {
        return Ok(config.system_prompt.clone());
    }
    let mut args = json!({});
    if let Some(s) = &config.system_prompt {
        args["system"] = json!(s);
    }
    if !input.skill_paths.is_empty() {
        args["skill_paths"] = json!(input.skill_paths);
    }
    if let Some(root) = &input.skill_root {
        args["skill_root"] = json!(root);
    }
    match ctx.call("prompt", &m("build"), args).await {
        Ok(msgs) => {
            let arr = msgs
                .as_array()
                .ok_or_else(|| Error::failed("prompt.build 非数组"))?;
            let sys = arr
                .iter()
                .find(|m| m.get("role") == Some(&json!("system")))
                .and_then(|m| m.get("content"))
                .and_then(|c| {
                    c.as_str()
                        .map(str::to_string)
                        .or_else(|| c.get("text").and_then(Value::as_str).map(str::to_string))
                });
            Ok(sys.or_else(|| config.system_prompt.clone()))
        }
        Err(e) => Err(e),
    }
}

async fn fetch_messages(ctx: &dyn AbilityCtx, session_id: &str) -> Result<Vec<Message>> {
    let out = ctx
        .call(
            "session",
            &m("protocol_messages"),
            json!({ "id": session_id }),
        )
        .await?;
    let mut messages: Vec<Message> =
        serde_json::from_value(out).map_err(|e| Error::failed(e.to_string()))?;
    messages = repair_tool_call_chain(messages);
    Ok(messages)
}

fn calls_for_event(calls: &[ToolCall]) -> Value {
    json!(
        calls
            .iter()
            .map(|c| json!({
                "id": c.id,
                "name": c.function.name,
                "arguments": c.function.arguments,
            }))
            .collect::<Vec<_>>()
    )
}

pub struct SpawnInput {
    pub session_id: String,
    pub prompt: String,
    pub summarize: bool,
}

pub struct SpawnOutput {
    pub parent_session_id: String,
    pub child_session_id: String,
    pub turn_id: String,
    pub reason: &'static str,
    pub text: String,
}

pub async fn spawn_turn(
    ctx: &dyn AbilityCtx,
    config: &TurnConfig,
    cancel: Arc<AtomicBool>,
    turn_id: &str,
    input: SpawnInput,
) -> Result<SpawnOutput> {
    let fork = ctx
        .call("session", &m("fork"), json!({ "id": input.session_id }))
        .await?;
    let child_id = fork
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::failed("session.fork 未返回 id"))?
        .to_string();

    let prompt = input.prompt.clone();
    let out = run_turn(
        ctx,
        config,
        cancel,
        turn_id,
        RunInput {
            session_id: Some(child_id.clone()),
            text: input.prompt,
            create_session: false,
            skill_paths: vec![],
            skill_root: None,
        },
    )
    .await?;

    let summary = if input.summarize && !out.text.is_empty() {
        out.text.clone()
    } else {
        format!(
            "[子代理完成] session={} reason={} rounds={}",
            child_id, out.reason, out.rounds
        )
    };

    ctx.call(
        "session",
        &m("append_user"),
        json!({
            "id": input.session_id,
            "text": format!("[子代理 {}] {}", child_id, prompt),
        }),
    )
    .await?;
    ctx.call(
        "session",
        &m("append_assistant"),
        json!({ "id": input.session_id, "text": summary }),
    )
    .await?;

    Ok(SpawnOutput {
        parent_session_id: input.session_id,
        child_session_id: child_id,
        turn_id: out.turn_id,
        reason: out.reason,
        text: summary,
    })
}
