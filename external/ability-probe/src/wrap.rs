//! `probe.wrap`：别名依赖 + typed client + 可逆副作用 + 发 `done` 供它方注入。

use std::sync::Arc;

use ability_sdk::prelude::*;
use ability_sdk::{AbilityCtx, Effect, Undo};
use async_trait::async_trait;

use crate::echo::EchoExt;

#[ability_sdk::client(id = "probe.wrap", ext = wrap)]
pub trait Wrap {
    async fn run(&self, args: Value) -> Result<Value>;
}

struct MarkEffect;

#[async_trait]
impl Effect for MarkEffect {
    async fn apply(&self, ctx: &dyn AbilityCtx) -> Result<Undo> {
        ctx.local_set("wrap.fx", json!(true)).await?;
        Ok(Undo::new(|| async { Ok(()) }))
    }
}

pub fn wrap_provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::must("probe.wrap")
        .depends_alias("probe.echo", "echo")
        .event_emit("done")
        .on(
            "run",
            "经别名调 echo，登记副作用，发 done",
            json!({ "type": "object" }),
            handler!(|ctx, args| {
                ctx.push_effect(Box::new(MarkEffect)).await?;
                let fx = ctx.local_get("wrap.fx").await?;
                let echoed = ctx.echo_as("echo").run(args.clone()).await?;
                emit!(ctx.done(json!({ "echo": echoed }))).await?;
                Ok(json!({ "wrapped": echoed, "fx": fx }))
            }),
        )
        .into_provider()
}
