use std::sync::Arc;

use ability_support::{AbilityCtx, Result};
use crate::bridge::PaidGw;
use serde_json::{Value, json};
use strata_shared::{failed, text_field};

pub async fn complete(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let _ = ctx;
    run(&crate::runtime::get()?.gateway(), &args).await
}

pub async fn run(gateway: &Arc<PaidGw>, args: &Value) -> Result<Value> {
    let receipt = text_field(args, "receiptId")?;
    gateway
        .complete(&receipt)
        .await
        .map_err(|e| failed(e.message))?;
    Ok(json!({}))
}
