//! `probe`：ping + emit pong。

use std::sync::Arc;

use ability_sdk::prelude::*;

pub fn provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::must("probe")
        .event_emit("pong")
        .on(
            "ping",
            "外部探针 ping",
            json!({ "type": "object" }),
            handler!(|ctx, _args| {
                emit!(ctx.pong(json!({ "from": "probe" }))).await?;
                Ok(json!({ "ok": true, "from": "external" }))
            }),
        )
        .into_provider()
}
