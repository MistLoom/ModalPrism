//! 业务逻辑：defs

use ability_support::{AbilityCtx, ProtocolKind, Result, method_catalog};
use serde_json::{Value, json};

use crate::bridge::catalog_to_tool_defs;

pub async fn defs(ctx: &dyn AbilityCtx, _args: Value) -> Result<Value> {
    let tools_ctx = ctx.with_protocol_filter(ProtocolKind::Tools);
    let methods = method_catalog(&tools_ctx.catalog());
    let defs = catalog_to_tool_defs(&methods);
    Ok(json!({ "tools": defs }))
}
