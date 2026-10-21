use std::sync::Arc;

use ability_support::{AbilityCtx, MethodId};
use async_trait::async_trait;
use planner::PlannerError;
use serde_json::json;

pub struct CtxPersist {
    ctx: Arc<dyn AbilityCtx>,
    owner: &'static str,
}

impl CtxPersist {
    pub fn new(ctx: Arc<dyn AbilityCtx>, owner: &'static str) -> Self {
        Self { ctx, owner }
    }

    fn method(name: &str) -> MethodId {
        MethodId::new(name).expect("method id")
    }
}

#[async_trait]
impl planner::Persist for CtxPersist {
    async fn load(&self, key: &str) -> Result<Option<Vec<u8>>, PlannerError> {
        if key != "state" {
            return Ok(None);
        }
        let value = self
            .ctx
            .call(
                "control_state",
                &Self::method("load"),
                json!({ "owner": self.owner, "key": key }),
            )
            .await
            .map_err(|e| PlannerError::new(e.to_string()))?;
        let payload = value.get("value").cloned().unwrap_or(serde_json::Value::Null);
        match payload {
            serde_json::Value::Null => Ok(None),
            serde_json::Value::String(text) => Ok(Some(text.into_bytes())),
            other => serde_json::to_vec(&other)
                .map(Some)
                .map_err(|e| PlannerError::new(e.to_string())),
        }
    }

    async fn store(&self, key: &str, bytes: &[u8]) -> Result<(), PlannerError> {
        if key != "state" {
            return Err(PlannerError::new("控制热数据读写失败"));
        }
        let payload: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|e| PlannerError::new(e.to_string()))?;
        self.ctx
            .call(
                "control_state",
                &Self::method("store"),
                json!({
                    "owner": self.owner,
                    "key": key,
                    "value": payload,
                }),
            )
            .await
            .map_err(|e| PlannerError::new(e.to_string()))?;
        Ok(())
    }
}
