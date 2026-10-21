use std::sync::Arc;

use ability_support::{AbilityCtx, MethodId};
use async_trait::async_trait;
use collector_ports::CtxCall;
use planner::PlannerError;
use serde_json::Value;

struct Bridge(Arc<dyn AbilityCtx>);

#[async_trait]
impl CtxCall for Bridge {
    async fn call(&self, ability: &str, method: &str, args: Value) -> Result<Value, PlannerError> {
        let method = MethodId::new(method).map_err(|e| PlannerError::new(e.to_string()))?;
        self.0
            .call(ability, &method, args)
            .await
            .map_err(|e| PlannerError::new(e.to_string()))
    }
}

pub fn wrap(ctx: Arc<dyn AbilityCtx>) -> Arc<dyn CtxCall> {
    Arc::new(Bridge(ctx))
}
