//! 经 Gateway 读取配置与布局。

use std::collections::BTreeMap;
use std::path::PathBuf;

use config::Snapshot;
use serde_json::Value;

use crate::error::{ConsoleError, ConsoleResult};
use crate::invoke;
use crate::ConsoleContext;

pub async fn applied(ctx: &ConsoleContext) -> ConsoleResult<Snapshot> {
    let value = invoke::call(&ctx.gateway, "config.applied", serde_json::json!({}))
        .await
        .map_err(|e| ConsoleError::internal(e))?;
    let generation = value
        .get("generation")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    let system = value.get("system").cloned().unwrap_or(Value::Null);
    let modules = value
        .get("modules")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    Ok(Snapshot {
        generation,
        system,
        modules,
        origins: BTreeMap::new(),
    })
}

pub async fn layout(ctx: &ConsoleContext) -> ConsoleResult<(PathBuf, PathBuf)> {
    let value = invoke::call(&ctx.gateway, "config.layout", serde_json::json!({}))
        .await
        .map_err(|e| ConsoleError::internal(e))?;
    let data = value
        .get("dataDir")
        .and_then(Value::as_str)
        .ok_or_else(|| ConsoleError::internal("布局无效"))?;
    let config = value
        .get("configDir")
        .and_then(Value::as_str)
        .ok_or_else(|| ConsoleError::internal("布局无效"))?;
    Ok((PathBuf::from(data), PathBuf::from(config)))
}
