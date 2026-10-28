//! `probe.hub`：生命周期、订阅、事件驱动调它方、自举 load/unload。
//!
//! Start：Manifest.listen `done` + 运行时订 `noted` / `host.load`，发 `ready`。
//! 收到它方事件后 `share` 出的 ctx 调 echo 并发 `caught`（事件注入）。
//! `boot` / `unboot`：`ctx.load` / `unload` `probe.note`。

use std::sync::Arc;

use crate::echo::EchoExt;
use ability_sdk::prelude::*;
use ability_sdk::{
    Ability, AbilityId, AbilityManifest, BusEvent, Dependency, EventClass, EventDecl, EventDir,
    EventFilter, FactoryProvider, InstanceId, LoadSpec, MethodId, MethodMeta, SourceId, host_event,
};
use async_trait::async_trait;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

pub fn hub_provider() -> Arc<dyn AbilityProvider> {
    FactoryProvider::new(hub_manifest(), |config| async move {
        Ok(Arc::new(Hub::new(config)) as Arc<dyn Ability>)
    })
}

fn hub_manifest() -> AbilityManifest {
    AbilityManifest {
        id: AbilityId::new("probe.hub").expect("id"),
        depends: vec![
            Dependency::required(AbilityId::new("probe.echo").expect("id")),
            Dependency::optional(AbilityId::new("probe.wrap").expect("id")),
        ],
        stateful: true,
        config_schema: json!({
            "type": "object",
            "properties": { "tag": { "type": "string" } }
        }),
        methods: vec![
            MethodMeta {
                id: MethodId::new("ping").expect("m"),
                description: "typed client 与 call! 调 echo".into(),
                input_schema: json!({ "type": "object" }),
                output_schema: None,
            },
            MethodMeta {
                id: MethodId::new("status").expect("m"),
                description: "config、绑定、目录、已收事件与注入结果".into(),
                input_schema: json!({ "type": "object" }),
                output_schema: None,
            },
            MethodMeta {
                id: MethodId::new("boot").expect("m"),
                description: "ctx.load probe.note".into(),
                input_schema: json!({ "type": "object" }),
                output_schema: None,
            },
            MethodMeta {
                id: MethodId::new("unboot").expect("m"),
                description: "ctx.unload probe.note".into(),
                input_schema: json!({ "type": "object" }),
                output_schema: None,
            },
        ],
        events: vec![
            EventDecl {
                name: "ready".into(),
                dir: EventDir::Emit,
                description: String::new(),
                payload_schema: json!({ "type": "object" }),
            },
            EventDecl {
                name: "caught".into(),
                dir: EventDir::Emit,
                description: String::new(),
                payload_schema: json!({ "type": "object" }),
            },
            EventDecl {
                name: "done".into(),
                dir: EventDir::Listen,
                description: String::new(),
                payload_schema: json!({ "type": "object" }),
            },
        ],
    }
}

struct Hub {
    manifest: AbilityManifest,
    config: Value,
    heard: Arc<Mutex<Vec<String>>>,
    injected: Arc<Mutex<Vec<Value>>>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

impl Hub {
    fn new(config: Value) -> Self {
        Self {
            manifest: hub_manifest(),
            config,
            heard: Arc::new(Mutex::new(Vec::new())),
            injected: Arc::new(Mutex::new(Vec::new())),
            tasks: Mutex::new(Vec::new()),
        }
    }

    async fn spawn_listen(
        &self,
        ctx: Arc<dyn AbilityCtx>,
        filter: EventFilter,
        react: bool,
    ) -> Result<()> {
        let mut sub = ctx.subscribe(filter).await?;
        let heard = Arc::clone(&self.heard);
        let injected = Arc::clone(&self.injected);
        let h = tokio::spawn(async move {
            while let Some(ev) = sub.next().await {
                heard.lock().await.push(ev.name().to_string());
                if !react {
                    continue;
                }
                let payload = match &ev {
                    BusEvent::Ability(a) => a.payload.clone(),
                    _ => continue,
                };
                let c: &dyn AbilityCtx = &*ctx;
                let Ok(out) = c
                    .echo()
                    .run(json!({ "injected": ev.name(), "from": payload }))
                    .await
                else {
                    continue;
                };
                injected.lock().await.push(out.clone());
                let _ = c.emit("caught", json!({ "echo": out })).await;
            }
        });
        self.tasks.lock().await.push(h);
        Ok(())
    }
}

#[async_trait]
impl Ability for Hub {
    fn manifest(&self) -> &AbilityManifest {
        &self.manifest
    }

    async fn on_lifecycle(&self, phase: Lifecycle, ctx: &dyn AbilityCtx) -> Result<()> {
        match phase {
            Lifecycle::Started => {
                let shared = ctx.share();
                // Manifest.listen：wrap.done → 调 echo 并发 caught
                self.spawn_listen(
                    Arc::clone(&shared),
                    EventFilter {
                        class: Some(EventClass::Ability),
                        name: Some("done".into()),
                        instance: None,
                    },
                    true,
                )
                .await?;
                // 运行时发现：note.noted（load 之后才有）
                self.spawn_listen(
                    Arc::clone(&shared),
                    EventFilter {
                        class: Some(EventClass::Ability),
                        name: Some("noted".into()),
                        instance: None,
                    },
                    true,
                )
                .await?;
                self.spawn_listen(
                    shared,
                    EventFilter {
                        class: Some(EventClass::Host),
                        name: Some(host_event::LOAD.into()),
                        instance: None,
                    },
                    false,
                )
                .await?;
                emit!(ctx.ready(json!({ "instance": ctx.instance().as_str() }))).await?;
                Ok(())
            }
            Lifecycle::Stopped => {
                let mut tasks = self.tasks.lock().await;
                for h in tasks.drain(..) {
                    h.abort();
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    async fn invoke(&self, method: &MethodId, ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
        match method.as_str() {
            "ping" => {
                let via_ext = ctx.echo().run(args.clone()).await?;
                let via_macro = call!(ctx.probe.echo::run(args)).await?;
                let hits = ctx.local_get("aspect.hits").await?;
                Ok(json!({
                    "via": via_ext.clone(),
                    "via_ext": via_ext,
                    "via_macro": via_macro,
                    "aspect_hits": hits,
                    "chain": ctx.chain_id(),
                    "span": ctx.span(),
                }))
            }
            "status" => {
                let heard = self.heard.lock().await.clone();
                let injected = self.injected.lock().await.clone();
                let echo = ctx.dep("probe.echo")?.to_string();
                let wrap = ctx.dep("probe.wrap").ok().map(|id| id.to_string());
                let catalog: Vec<String> = ctx
                    .catalog()
                    .into_iter()
                    .map(|e| e.instance.to_string())
                    .collect();
                Ok(json!({
                    "tag": self.config.get("tag"),
                    "heard": heard,
                    "injected": injected,
                    "echo": echo,
                    "wrap": wrap,
                    "catalog": catalog,
                }))
            }
            "boot" => {
                let id = ctx
                    .load(
                        LoadSpec::new(
                            SourceId::new("probe.note").expect("src"),
                            InstanceId::new("probe.note").expect("iid"),
                        )
                        .with_config(json!({ "label": "from-hub" })),
                    )
                    .await?;
                Ok(json!({ "loaded": id.to_string() }))
            }
            "unboot" => {
                ctx.unload(&InstanceId::new("probe.note").expect("iid"))
                    .await?;
                Ok(json!({ "unloaded": "probe.note" }))
            }
            _ => Err(Error::not_found(method)),
        }
    }
}
