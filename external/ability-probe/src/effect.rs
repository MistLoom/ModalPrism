//! `probe.effect`：invoke 失败时在边界撤销 push_effect。

use std::sync::Arc;

use ability_sdk::prelude::*;
use ability_sdk::{Effect, Undo};
use async_trait::async_trait;

struct FxEffect;

#[async_trait]
impl Effect for FxEffect {
    async fn apply(&self, ctx: &dyn AbilityCtx) -> Result<Undo> {
        ctx.local_set("fx", json!(true)).await?;
        let kept = ctx.retain();
        Ok(Undo::new(move || async move {
            kept.local_set("fx", Value::Null).await
        }))
    }
}

pub fn effect_provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::must("probe.effect")
        .stateful()
        .on(
            "run",
            "fail after effect",
            json!({ "type": "object" }),
            handler!(|ctx, _args| {
                ctx.push_effect(Box::new(FxEffect)).await?;
                Err(Error::failed("probe.effect.fail"))
            }),
        )
        .on(
            "peek",
            "read local fx",
            json!({ "type": "object" }),
            handler!(|ctx, _args| {
                let fx = ctx.local_get("fx").await?;
                Ok(json!({ "fx": fx }))
            }),
        )
        .into_provider()
}
