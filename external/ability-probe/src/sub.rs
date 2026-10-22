//! `probe.sub`：Started 时 subscribe，ping 里 emit 再读到同一条事件。

use std::sync::Arc;

use ability_sdk::prelude::*;
use ability_sdk::{
    AbilityCtx, EventClass, EventFilter, EventListener, Lifecycle, box_fut, box_life_fut,
};
use tokio::sync::Mutex;

pub fn sub_provider() -> Arc<dyn AbilityProvider> {
    let slot: Arc<Mutex<Option<Box<dyn EventListener + Send>>>> = Arc::new(Mutex::new(None));
    let slot_life = Arc::clone(&slot);
    let slot_ping = Arc::clone(&slot);
    AbilitySpec::must("probe.sub")
        .stateful()
        .event_emit("tick")
        .on_lifecycle({
            let slot_life = Arc::clone(&slot_life);
            move |phase: Lifecycle, ctx: &dyn AbilityCtx| {
                let slot_life = Arc::clone(&slot_life);
                box_life_fut(async move {
                    if phase == Lifecycle::Started {
                        let sub = ctx
                            .subscribe(EventFilter {
                                class: Some(EventClass::Ability),
                                name: Some("tick".into()),
                                instance: Some(ctx.instance().clone()),
                            })
                            .await?;
                        *slot_life.lock().await = Some(sub);
                    }
                    Ok(())
                })
            }
        })
        .on(
            "ping",
            "emit tick and read subscribe",
            json!({ "type": "object" }),
            {
                let slot_ping = Arc::clone(&slot_ping);
                move |ctx: &dyn AbilityCtx, _args: Value| {
                    let slot_ping = Arc::clone(&slot_ping);
                    box_fut(async move {
                        ctx.emit("tick", json!({ "n": 1 })).await?;
                        let mut guard = slot_ping.lock().await;
                        let sub = guard.as_mut().expect("subscribe");
                        let ev = sub.next().await;
                        let name = ev.as_ref().map(|e| e.name().to_string());
                        Ok(json!({ "got": name }))
                    })
                }
            },
        )
        .into_provider()
}
