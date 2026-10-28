//! 单一 dispatch 切面：合并 CC/DSH 生态工具。

use std::sync::Arc;

use ability_support::{AbilityCtx, Aspect, Error, Next, Result};
use ai_protocol::{ToolCall, ToolResultMessage, json_content};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::bridge::PluginHostBridge;
use crate::coordinator::PluginHostCoordinator;
use crate::naming::parse_llm_tool_name;

pub struct EcosystemDispatchAspect {
    coord: Arc<PluginHostCoordinator>,
}

impl EcosystemDispatchAspect {
    pub fn new(coord: Arc<PluginHostCoordinator>) -> Self {
        Self { coord }
    }
}

#[async_trait]
impl Aspect for EcosystemDispatchAspect {
    fn order(&self) -> i32 {
        10
    }

    async fn around(&self, ctx: &dyn AbilityCtx, args: &Value, next: Next<'_>) -> Result<Value> {
        let inst = ctx.instance().as_str();
        let method = ctx.method().map(|m| m.as_str()).unwrap_or("");
        if inst != "dispatch" {
            return next.run().await;
        }
        match method {
            "defs" => merge_defs(self, next).await,
            "run" => merge_run(self, args, next).await,
            _ => next.run().await,
        }
    }
}

async fn merge_defs(aspect: &EcosystemDispatchAspect, next: Next<'_>) -> Result<Value> {
    let base = next.run().await?;
    let bridge = aspect
        .coord
        .bridge()
        .await
        .ok_or_else(|| Error::failed("插件宿主未启动"))?;
    let mut tools = base
        .get("tools")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    for frame in bridge.registered_tool_defs() {
        tools.push(frame);
    }
    Ok(json!({ "tools": tools }))
}

#[derive(Deserialize)]
struct RunArgs {
    calls: Vec<ToolCall>,
    #[serde(default)]
    continue_on_error: bool,
}

async fn merge_run(
    aspect: &EcosystemDispatchAspect,
    args: &Value,
    next: Next<'_>,
) -> Result<Value> {
    let parsed: RunArgs = serde_json::from_value(args.clone())
        .map_err(|e| ability_support::Error::invalid_args(e.to_string()))?;
    let bridge = aspect.coord.bridge().await;
    let tool_check = aspect.coord.tool_check_enabled().await;
    let mut native = Vec::new();
    let mut plugin_results: Vec<Value> = Vec::new();
    for call in &parsed.calls {
        if tool_check {
            let b = bridge
                .as_ref()
                .ok_or_else(|| Error::failed("插件宿主未启动"))?;
            run_tool_check(b, call).await?;
        }
        if let Some(host_key) = parse_llm_tool_name(&call.function.name) {
            let b = bridge
                .as_ref()
                .ok_or_else(|| Error::failed("插件宿主未启动"))?;
            let args: Value = call
                .function
                .parse_args()
                .map_err(|e| Error::invalid_args(e.to_string()))?;
            match b.execute_tool(&host_key, args).await {
                Ok(data) => {
                    let msg = ToolResultMessage {
                        tool_call_id: call.id.clone(),
                        content: json_content(&data),
                    };
                    plugin_results
                        .push(serde_json::to_value(msg.into_message()).unwrap_or(json!({})));
                }
                Err(err) if parsed.continue_on_error => {
                    let msg = ToolResultMessage {
                        tool_call_id: call.id.clone(),
                        content: json!({ "error": err.to_string() }).to_string(),
                    };
                    plugin_results
                        .push(serde_json::to_value(msg.into_message()).unwrap_or(json!({})));
                }
                Err(err) => return Err(err),
            }
        } else {
            native.push(call.clone());
        }
    }
    if native.is_empty() {
        return Ok(json!({ "messages": plugin_results }));
    }
    let mut rem_args = args.clone();
    if let Some(obj) = rem_args.as_object_mut() {
        obj.insert(
            "calls".into(),
            serde_json::to_value(&native).unwrap_or(json!([])),
        );
    }
    let native_out = next.with_args(rem_args).await?;
    let mut messages = plugin_results;
    if let Some(arr) = native_out.get("messages").and_then(|v| v.as_array()) {
        for m in arr {
            messages.push(m.clone());
        }
    }
    Ok(json!({ "messages": messages }))
}

async fn run_tool_check(bridge: &PluginHostBridge, call: &ToolCall) -> Result<()> {
    let tool_input = call
        .function
        .parse_args()
        .map_err(|e| Error::invalid_args(e.to_string()))?;
    let payload = json!({
        "tool_call_id": call.id,
        "tool_name": call.function.name,
        "tool_input": tool_input,
    });
    let out = bridge.call_hook("tool.check", payload).await?;
    if out.get("block").and_then(|v| v.as_bool()) == Some(true) {
        let msg = out
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("tool.check 拒绝执行");
        return Err(Error::failed(msg));
    }
    Ok(())
}
