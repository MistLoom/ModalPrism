//! `probe.chain`：middleware 调用 next 后置位。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ability_sdk::prelude::*;
use ability_sdk::{
    BusEvent, EventClass, EventFilter, EventHandler, EventMiddleware, EventNext, EventOpts,
};
use async_trait::async_trait;

pub(crate) struct BitMw(pub Arc<AtomicBool>);

#[async_trait]
impl EventMiddleware for BitMw {
    async fn around(&self, event: BusEvent, next: EventNext) -> Result<BusEvent> {
        let ev = next.run(event).await?;
        self.0.store(true, Ordering::SeqCst);
        Ok(ev)
    }
}

pub fn chain_provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::must("probe.chain")
        .stateful()
        .event_emit("chain")
        .on(
            "run",
            "middleware chain",
            json!({ "type": "object" }),
            handler!(|ctx, _args| {
                let ran = Arc::new(AtomicBool::new(false));
                let _reg = ctx
                    .handle(
                        EventFilter {
                            class: Some(EventClass::Ability),
                            name: Some("chain".into()),
                            instance: None,
                        },
                        EventHandler::Middleware(Arc::new(BitMw(Arc::clone(&ran)))),
                        EventOpts::default(),
                    )
                    .await?;
                ctx.emit("chain", json!({ "step": 1 })).await?;
                Ok(json!({ "middleware_ran": ran.load(Ordering::SeqCst) }))
            }),
        )
        .into_provider()
}
