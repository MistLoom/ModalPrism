//! 业务逻辑：run

use ability_support::{AbilityCtx, Error, ProtocolKind, Result, method_catalog};
use ai_protocol::{ToolCall, ToolResultMessage, json_content};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::map::NameMap;

#[derive(Deserialize)]
struct RunArgs {
    calls: Vec<ToolCall>,
    #[serde(default)]
    continue_on_error: bool,
}

async fn dispatch_one(
    ctx: &dyn AbilityCtx,
    names: &NameMap,
    call: &ToolCall,
) -> Result<ToolResultMessage> {
    let (inst, method) = names.resolve(&call.function.name)?;
    let args: Value = call
        .function
        .parse_args()
        .map_err(|e| Error::invalid_args(e.to_string()))?;
    let data = ctx.call(inst.as_str(), method, args).await?;
    Ok(ToolResultMessage {
        tool_call_id: call.id.clone(),
        content: json_content(&data),
    })
}

pub async fn run(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let a: RunArgs =
        serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
    let tools_ctx = ctx.with_protocol_filter(ProtocolKind::Tools);
    let names = NameMap::from_catalog(&method_catalog(&tools_ctx.catalog()));
    let mut out = Vec::new();
    for call in &a.calls {
        match dispatch_one(&*tools_ctx, &names, call).await {
            Ok(msg) => out.push(msg.into_message()),
            Err(err) if a.continue_on_error => {
                out.push(
                    ToolResultMessage {
                        tool_call_id: call.id.clone(),
                        content: json!({ "error": err.to_string() }).to_string(),
                    }
                    .into_message(),
                );
            }
            Err(err) => return Err(err),
        }
    }
    Ok(json!({ "messages": out }))
}
