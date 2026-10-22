//! `probe.ctx`：share 与 chain_status。

use std::sync::Arc;

use ability_sdk::prelude::*;

pub fn ctx_provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::must("probe.ctx")
        .on(
            "run",
            "share and chain_status",
            json!({ "type": "object" }),
            handler!(|ctx, _args| {
                let shared = ctx.share();
                let status = ctx.chain_status(None);
                Ok(json!({
                    "has_shared": true,
                    "chain": ctx.chain_id(),
                    "status": status,
                    "shared_instance": shared.instance().as_str(),
                }))
            }),
        )
        .into_provider()
}
