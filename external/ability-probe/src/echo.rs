//! `probe.echo`：回显 + client 契约。

use std::sync::Arc;

use ability_sdk::prelude::*;

#[ability_sdk::client(id = "probe.echo", ext = echo)]
pub trait Echo {
    async fn run(&self, args: Value) -> Result<Value>;
}

pub fn echo_provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::must("probe.echo")
        .on(
            "run",
            "回显",
            json!({ "type": "object" }),
            handler!(|_ctx, args| Ok(args)),
        )
        .into_provider()
}
