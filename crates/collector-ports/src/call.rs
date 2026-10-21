use async_trait::async_trait;
use serde_json::Value;

use planner::PlannerError;

/// 经 Gateway 调用其他 Ability，不依赖 `ability-support`。
#[async_trait]
pub trait CtxCall: Send + Sync {
    async fn call(&self, ability: &str, method: &str, args: Value) -> Result<Value, PlannerError>;
}
