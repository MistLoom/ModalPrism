//! `probe.surface`：四种事件模式、协议过滤仍共享 locals、register_aspect 失败。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ability_sdk::prelude::*;
use ability_sdk::{
    Aspect, BusEvent, EventClass, EventFilter, EventHandler, EventOpts, EventPipeline, EventSink,
    Next, ProtocolKind,
};
use async_trait::async_trait;

use crate::chain::BitMw;

struct BitPipe(Arc<AtomicBool>);

#[async_trait]
impl EventPipeline for BitPipe {
    async fn process(&self, event: BusEvent) -> Result<BusEvent> {
        self.0.store(true, Ordering::SeqCst);
        Ok(event)
    }
}

struct BitSink(Arc<AtomicBool>);

#[async_trait]
impl EventSink for BitSink {
    async fn on_event(&self, _event: &BusEvent) {
        self.0.store(true, Ordering::SeqCst);
    }
}

struct NoAspect;

#[async_trait]
impl Aspect for NoAspect {
    async fn around(&self, _ctx: &dyn AbilityCtx, _args: &Value, next: Next<'_>) -> Result<Value> {
        next.run().await
    }
}

fn lane_filter() -> EventFilter {
    EventFilter {
        class: Some(EventClass::Ability),
        name: Some("lane".into()),
        instance: None,
    }
}

pub fn surface_provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::must("probe.surface")
        .stateful()
        .event_emit("lane")
        .on(
            "lanes",
            "四种事件模式",
            json!({ "type": "object" }),
            handler!(|ctx, _args| {
                let mw = Arc::new(AtomicBool::new(false));
                let pipe = Arc::new(AtomicBool::new(false));
                let par = Arc::new(AtomicBool::new(false));
                let msg = Arc::new(AtomicBool::new(false));
                let _mw = ctx
                    .handle(
                        lane_filter(),
                        EventHandler::Middleware(Arc::new(BitMw(Arc::clone(&mw)))),
                        EventOpts::default(),
                    )
                    .await?;
                let _pipe = ctx
                    .handle(
                        lane_filter(),
                        EventHandler::Pipeline(Arc::new(BitPipe(Arc::clone(&pipe)))),
                        EventOpts::default(),
                    )
                    .await?;
                let _par = ctx
                    .handle(
                        lane_filter(),
                        EventHandler::Parallel(Arc::new(BitSink(Arc::clone(&par)))),
                        EventOpts::default(),
                    )
                    .await?;
                let _msg = ctx
                    .handle(
                        lane_filter(),
                        EventHandler::Message(Arc::new(BitSink(Arc::clone(&msg)))),
                        EventOpts::default(),
                    )
                    .await?;
                ctx.emit("lane", json!({})).await?;
                // message 由宿主投递后不等待，这里抽干后续帧再读标志。
                for _ in 0..40 {
                    if msg.load(Ordering::SeqCst) {
                        break;
                    }
                    let _ = ctx.local_get("_drain").await?;
                }
                Ok(json!({
                    "middleware": mw.load(Ordering::SeqCst),
                    "pipeline": pipe.load(Ordering::SeqCst),
                    "parallel": par.load(Ordering::SeqCst),
                    "message": msg.load(Ordering::SeqCst),
                }))
            }),
        )
        .on(
            "filter",
            "with_protocol_filter 仍是同一 ctx",
            json!({ "type": "object" }),
            handler!(|ctx, _args| {
                let filtered = ctx.with_protocol_filter(ProtocolKind::Fun);
                filtered.local_set("pf", json!(1)).await?;
                let v = ctx.local_get("pf").await?;
                Ok(json!({
                    "shared_local": v == Some(json!(1)),
                    "same_instance": filtered.instance().as_str() == ctx.instance().as_str(),
                }))
            }),
        )
        .on(
            "aspect",
            "register_aspect 失败",
            json!({ "type": "object" }),
            handler!(|ctx, _args| {
                ctx.register_aspect(Arc::new(NoAspect))?;
                Ok(json!({}))
            }),
        )
        .into_provider()
}
