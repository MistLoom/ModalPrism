//! 业务逻辑：run

use ability_support::{AbilityCtx, Result};
use serde_json::Value;

pub async fn run(_ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    Ok(args)
}
