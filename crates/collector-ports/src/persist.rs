use std::sync::Arc;

use async_trait::async_trait;
use planner::PlannerError;
use serde_json::json;

use crate::call::CtxCall;

pub struct CtxPersist {
    ctx: Arc<dyn CtxCall>,
    owner: &'static str,
}

impl CtxPersist {
    pub fn new(ctx: Arc<dyn CtxCall>, owner: &'static str) -> Self {
        Self { ctx, owner }
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
                "load",
                json!({ "owner": self.owner, "key": key }),
            )
            .await?;
        let payload = value.get("value").cloned().unwrap_or(serde_json::Value::Null);
        match payload {
            serde_json::Value::Null => Ok(None),
            serde_json::Value::String(text) => Ok(Some(text.into_bytes())),
            other => Ok(Some(other.to_string().into_bytes())),
        }
    }

    async fn store(&self, key: &str, value: &[u8]) -> Result<(), PlannerError> {
        if key != "state" {
            return Ok(());
        }
        let payload = String::from_utf8_lossy(value).to_string();
        self.ctx
            .call(
                "control_state",
                "store",
                json!({ "owner": self.owner, "key": key, "value": payload }),
            )
            .await?;
        Ok(())
    }
}
