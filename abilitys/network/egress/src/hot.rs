use std::sync::Arc;

use ability_support::{AbilityCtx, MethodId};
use async_trait::async_trait;
use serde_json::{Value, json};

#[derive(Clone)]
pub struct Hot {
    ctx: Arc<dyn AbilityCtx>,
    owner: &'static str,
}

impl Hot {
    pub fn new(ctx: Arc<dyn AbilityCtx>, owner: &'static str) -> Self {
        Self { ctx, owner }
    }
}

async fn load_bytes(hot: &Hot) -> Result<Option<Vec<u8>>, String> {
    let method = MethodId::new("load").map_err(|e| e.to_string())?;
    let response = hot
        .ctx
        .call(
            "control_state",
            &method,
            json!({ "owner": hot.owner, "key": "state" }),
        )
        .await
        .map_err(|e| e.to_string())?;
    match response.get("value") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => Ok(Some(value.to_string().into_bytes())),
    }
}

async fn store_bytes(hot: &Hot, bytes: &[u8]) -> Result<(), String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let method = MethodId::new("store").map_err(|e| e.to_string())?;
    hot.ctx
        .call(
            "control_state",
            &method,
            json!({
                "owner": hot.owner,
                "key": "state",
                "value": value,
            }),
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[async_trait]
impl egress::Persist for Hot {
    async fn load(&self, key: &str) -> Result<Option<Vec<u8>>, accessors::AccessError> {
        if key != "state" {
            return Ok(None);
        }
        load_bytes(self).await.map_err(|error| {
            accessors::AccessError::with_source(
                strata_model::ErrorClass::Transient,
                "控制热数据读写失败",
                std::io::Error::other(error),
            )
        })
    }

    async fn store(&self, key: &str, bytes: &[u8]) -> Result<(), accessors::AccessError> {
        if key != "state" {
            return Err(accessors::AccessError::new(
                strata_model::ErrorClass::Transient,
                "控制热数据读写失败",
            ));
        }
        store_bytes(self, bytes).await.map_err(|error| {
            accessors::AccessError::with_source(
                strata_model::ErrorClass::Transient,
                "控制热数据读写失败",
                std::io::Error::other(error),
            )
        })
    }
}
