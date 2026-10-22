mod common;

use std::sync::Arc;

use ability_core::AbilityHost;
use ability_support::{
    Ability, CtxFnAbility, EventClass, EventFilter, FnAbility, InstanceId, LoadSpec, MethodId,
    SourceId, StaticProvider, UserEvent, UserEventSink,
};
use frame_core::InvokeRequest;
use serde_json::json;

#[tokio::test]
async fn ability_cannot_emit_undeclared() {
    let host = AbilityHost::new();
    let m = common::manifest("x", &["run"]);
    let ability: Arc<dyn Ability> = Arc::new(FnAbility::new(m, |a| async move { Ok(a) }));
    host.install_str("x", Arc::new(StaticProvider::new(ability)))
        .await
        .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("x").unwrap(),
        InstanceId::new("x").unwrap(),
    ))
    .await
    .unwrap();

    let m2 = common::manifest_with_events("y", &["run"], &[]);
    let y: Arc<dyn Ability> = Arc::new(CtxFnAbility::new(m2, |ctx, _| {
        Box::pin(async move {
            ctx.emit("nope", json!({})).await?;
            Ok(json!({}))
        })
    }));
    host.install_str("y", Arc::new(StaticProvider::new(y)))
        .await
        .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("y").unwrap(),
        InstanceId::new("y").unwrap(),
    ))
    .await
    .unwrap();
    let err = host
        .clone()
        .into_gateway()
        .invoke(InvokeRequest::new(
            InstanceId::new("y").unwrap(),
            MethodId::new("run").unwrap(),
            json!({}),
        ))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("未声明"));
}

#[tokio::test]
async fn ability_cannot_emit_invalid_payload() {
    let host = AbilityHost::new();
    let schema = json!({
        "type": "object",
        "properties": { "n": { "type": "integer" } },
        "required": ["n"]
    });
    let mut m = common::manifest("z", &["run"]);
    m.events = vec![ability_support::EventDecl {
        name: "tick".into(),
        dir: ability_support::EventDir::Emit,
        description: String::new(),
        payload_schema: schema,
    }];
    let ability: Arc<dyn Ability> = Arc::new(CtxFnAbility::new(m, |ctx, _| {
        Box::pin(async move {
            ctx.emit("tick", json!({ "n": "not_int" })).await?;
            Ok(json!({}))
        })
    }));
    host.install_str("z", Arc::new(StaticProvider::new(ability)))
        .await
        .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("z").unwrap(),
        InstanceId::new("z").unwrap(),
    ))
    .await
    .unwrap();
    let err = host
        .clone()
        .into_gateway()
        .invoke(InvokeRequest::new(
            InstanceId::new("z").unwrap(),
            MethodId::new("run").unwrap(),
            json!({}),
        ))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("payload_schema"));
}

#[tokio::test]
async fn late_joiner_catalog_and_subscribe() {
    let host = AbilityHost::new();
    host.install_str("echo", common::echo_provider("echo"))
        .await
        .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("echo").unwrap(),
        InstanceId::new("echo").unwrap(),
    ))
    .await
    .unwrap();

    let seen = Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
    let seen2 = Arc::clone(&seen);
    let m = common::manifest("late", &["run"]);
    let late: Arc<dyn Ability> = Arc::new(CtxFnAbility::new(m, move |ctx, _| {
        let seen2 = Arc::clone(&seen2);
        Box::pin(async move {
            let cat = ctx.catalog();
            assert!(cat.iter().any(|e| e.instance.as_str() == "echo"));
            let mut sub = ctx
                .subscribe(EventFilter {
                    class: Some(EventClass::Host),
                    name: Some("host.load".into()),
                    instance: None,
                })
                .await?;
            ctx.load(LoadSpec::new(
                SourceId::new("echo").unwrap(),
                InstanceId::new("echo2").unwrap(),
            ))
            .await?;
            if let Some(ev) = sub.next().await {
                seen2.lock().await.push(ev.name().to_string());
            }
            Ok(json!({ "ok": true }))
        })
    }));
    host.install_str("late", Arc::new(StaticProvider::new(late)))
        .await
        .unwrap();
    host.install_str("echo", common::echo_provider("echo"))
        .await
        .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("late").unwrap(),
        InstanceId::new("late").unwrap(),
    ))
    .await
    .unwrap();

    host.clone()
        .into_gateway()
        .invoke(InvokeRequest::new(
            InstanceId::new("late").unwrap(),
            MethodId::new("run").unwrap(),
            json!({}),
        ))
        .await
        .unwrap();
    let names = seen.lock().await;
    assert!(names.iter().any(|n| n == "host.load"));
}

#[tokio::test]
async fn user_event_only_from_host() {
    let host = AbilityHost::new();
    let got = Arc::new(tokio::sync::Mutex::new(None));
    struct S(Arc<tokio::sync::Mutex<Option<String>>>);
    #[async_trait::async_trait]
    impl UserEventSink for S {
        async fn on_user_event(&self, event: &UserEvent) {
            *self.0.lock().await = Some(event.name.clone());
        }
    }
    host.events()
        .subscribe_user(Arc::new(S(Arc::clone(&got))))
        .await;
    host.emit_user(UserEvent::new("mark", json!({}))).await;
    assert_eq!(got.lock().await.as_deref(), Some("mark"));
}

#[tokio::test]
async fn handle_order_pipeline_middleware_short_circuit() {
    use ability_core::EventBus;
    use ability_support::{
        AbilityEvent, EventHandler, EventMiddleware, EventNext, EventOpts, EventPipeline,
        EventSink, InstanceId, Result,
    };
    use tokio::sync::Mutex;

    let bus = EventBus::new();
    let log = Arc::new(Mutex::new(Vec::<String>::new()));

    struct Pipe(Arc<Mutex<Vec<String>>>, &'static str);
    #[async_trait::async_trait]
    impl EventPipeline for Pipe {
        async fn process(
            &self,
            mut event: ability_support::BusEvent,
        ) -> Result<ability_support::BusEvent> {
            self.0.lock().await.push(self.1.into());
            let n = event.payload()["n"].as_i64().unwrap_or(0);
            event.payload_mut()["n"] = json!(n + 1);
            Ok(event)
        }
    }

    struct Mw {
        log: Arc<Mutex<Vec<String>>>,
        tag: &'static str,
        block: bool,
    }
    #[async_trait::async_trait]
    impl EventMiddleware for Mw {
        async fn around(
            &self,
            event: ability_support::BusEvent,
            next: EventNext,
        ) -> Result<ability_support::BusEvent> {
            self.log.lock().await.push(format!("{}-in", self.tag));
            if self.block {
                return Ok(event);
            }
            let ev = next.run(event).await?;
            self.log.lock().await.push(format!("{}-out", self.tag));
            Ok(ev)
        }
    }

    struct Msg(Arc<Mutex<Vec<String>>>);
    #[async_trait::async_trait]
    impl EventSink for Msg {
        async fn on_event(&self, event: &ability_support::BusEvent) {
            self.0
                .lock()
                .await
                .push(format!("msg:{}", event.payload()["n"]));
        }
    }

    let _p1 = bus.handle(
        EventFilter::default(),
        EventHandler::Pipeline(Arc::new(Pipe(Arc::clone(&log), "p1"))),
        EventOpts::default(),
    );
    let _p0 = bus.handle(
        EventFilter::default(),
        EventHandler::Pipeline(Arc::new(Pipe(Arc::clone(&log), "p0"))),
        EventOpts::prepend(),
    );
    let _a = bus.handle(
        EventFilter::default(),
        EventHandler::Middleware(Arc::new(Mw {
            log: Arc::clone(&log),
            tag: "a",
            block: false,
        })),
        EventOpts::default(),
    );
    let _b = bus.handle(
        EventFilter::default(),
        EventHandler::Middleware(Arc::new(Mw {
            log: Arc::clone(&log),
            tag: "b",
            block: false,
        })),
        EventOpts::prepend(),
    );
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let _m = bus.handle(
        EventFilter::default(),
        EventHandler::Message(Arc::new(Msg(Arc::clone(&seen)))),
        EventOpts::default(),
    );
    let _m2 = bus.handle(
        EventFilter::default(),
        EventHandler::Message(Arc::new(Msg(Arc::clone(&seen)))),
        EventOpts::default(),
    );

    bus.emit_ability(AbilityEvent {
        instance: InstanceId::new("x").unwrap(),
        name: "e".into(),
        payload: json!({ "n": 0 }),
        at: 1,
    })
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;

    let steps = log.lock().await.clone();
    assert_eq!(steps, vec!["b-in", "a-in", "p0", "p1", "a-out", "b-out"]);
    let got = seen.lock().await.clone();
    assert_eq!(got.len(), 2);
    assert!(got.iter().all(|s| s == "msg:2"));

    let log2 = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen2 = Arc::new(Mutex::new(Vec::<String>::new()));
    let bus2 = EventBus::new();
    let _block = bus2.handle(
        EventFilter::default(),
        EventHandler::Middleware(Arc::new(Mw {
            log: Arc::clone(&log2),
            tag: "k",
            block: true,
        })),
        EventOpts::default(),
    );
    let _msg = bus2.handle(
        EventFilter::default(),
        EventHandler::Message(Arc::new(Msg(Arc::clone(&seen2)))),
        EventOpts::default(),
    );
    bus2.emit_ability(AbilityEvent {
        instance: InstanceId::new("x").unwrap(),
        name: "e".into(),
        payload: json!({ "n": 0 }),
        at: 1,
    })
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert_eq!(*log2.lock().await, vec!["k-in"]);
    assert!(seen2.lock().await.is_empty());
}
