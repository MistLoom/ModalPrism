//! 三路事件总线。Host 构造后即可用。
//!
//! Host / 能力：middleware ⊃ Pipeline → 分叉 parallel（汇合）+ message（不等待）。
//! 用户事件仍走通道。

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use ability_support::{
    AbilityEvent, BusEvent, EventFilter, EventHandler, EventListener, EventMiddleware, EventNext,
    EventOpts, EventPipeline, EventReg, EventSink, EventUnreg, HostEvent, Result, UserEvent,
    UserEventSink,
};
use async_trait::async_trait;
use serde::Serialize;
use tokio::sync::mpsc;

pub struct ChannelListener {
    rx: mpsc::UnboundedReceiver<BusEvent>,
    _reg: EventReg,
}

impl ChannelListener {
    pub fn id(&self) -> u64 {
        self._reg.id()
    }
}

#[async_trait]
impl EventListener for ChannelListener {
    async fn next(&mut self) -> Option<BusEvent> {
        self.rx.recv().await
    }
}

struct FilterSink {
    tx: mpsc::UnboundedSender<BusEvent>,
    allow: Arc<dyn Fn(&BusEvent) -> bool + Send + Sync>,
}

#[async_trait]
impl EventSink for FilterSink {
    async fn on_event(&self, event: &BusEvent) {
        if !(self.allow)(event) {
            return;
        }
        let _ = self.tx.send(event.clone());
    }
}

struct Slot<T> {
    id: u64,
    filter: EventFilter,
    handler: T,
}

struct Inner {
    next_id: u64,
    middleware: Vec<Slot<Arc<dyn EventMiddleware>>>,
    pipeline: Vec<Slot<Arc<dyn EventPipeline>>>,
    parallel: Vec<Slot<Arc<dyn EventSink>>>,
    message: Vec<Slot<Arc<dyn EventSink>>>,
    user_sinks: Vec<Arc<dyn UserEventSink>>,
}

impl Inner {
    fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn insert<T>(lane: &mut Vec<Slot<T>>, slot: Slot<T>, prepend: bool) {
        if prepend {
            lane.insert(0, slot);
        } else {
            lane.push(slot);
        }
    }

    fn unregister(&mut self, id: u64) {
        self.middleware.retain(|s| s.id != id);
        self.pipeline.retain(|s| s.id != id);
        self.parallel.retain(|s| s.id != id);
        self.message.retain(|s| s.id != id);
    }
}

struct Snap {
    mw: Vec<Arc<dyn EventMiddleware>>,
    pipeline: Vec<Arc<dyn EventPipeline>>,
    parallel: Vec<Arc<dyn EventSink>>,
    message: Vec<Arc<dyn EventSink>>,
}

#[derive(Clone)]
pub struct EventBus {
    inner: Arc<Mutex<Inner>>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                next_id: 1,
                middleware: Vec::new(),
                pipeline: Vec::new(),
                parallel: Vec::new(),
                message: Vec::new(),
                user_sinks: Vec::new(),
            })),
        }
    }
}

impl EventUnreg for EventBus {
    fn unregister(&self, id: u64) {
        self.inner.lock().expect("events").unregister(id);
    }
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    fn unreg_handle(&self) -> Arc<dyn EventUnreg> {
        Arc::new(self.clone())
    }

    pub fn handle(&self, filter: EventFilter, handler: EventHandler, opts: EventOpts) -> EventReg {
        let mut g = self.inner.lock().expect("events");
        let id = g.take_id();
        match handler {
            EventHandler::Middleware(h) => Inner::insert(
                &mut g.middleware,
                Slot {
                    id,
                    filter,
                    handler: h,
                },
                opts.prepend,
            ),
            EventHandler::Pipeline(h) => Inner::insert(
                &mut g.pipeline,
                Slot {
                    id,
                    filter,
                    handler: h,
                },
                opts.prepend,
            ),
            EventHandler::Parallel(h) => Inner::insert(
                &mut g.parallel,
                Slot {
                    id,
                    filter,
                    handler: h,
                },
                opts.prepend,
            ),
            EventHandler::Message(h) => Inner::insert(
                &mut g.message,
                Slot {
                    id,
                    filter,
                    handler: h,
                },
                opts.prepend,
            ),
        }
        drop(g);
        EventReg::new(id, self.unreg_handle())
    }

    pub fn subscribe_sink(&self, sink: Arc<dyn EventSink>) -> EventReg {
        self.handle(
            EventFilter::default(),
            EventHandler::Message(sink),
            EventOpts::default(),
        )
    }

    pub async fn subscribe_user(&self, sink: Arc<dyn UserEventSink>) {
        self.inner.lock().expect("events").user_sinks.push(sink);
    }

    pub async fn listen(&self, filter: EventFilter) -> ChannelListener {
        self.listen_where(filter, Arc::new(|_: &BusEvent| true))
            .await
    }

    /// 与 [`listen`] 相同，额外丢弃 `allow` 为假的事件。
    pub async fn listen_where(
        &self,
        filter: EventFilter,
        allow: Arc<dyn Fn(&BusEvent) -> bool + Send + Sync>,
    ) -> ChannelListener {
        let (tx, rx) = mpsc::unbounded_channel();
        let reg = self.handle(
            filter,
            EventHandler::Message(Arc::new(FilterSink { tx, allow })),
            EventOpts::default(),
        );
        ChannelListener { rx, _reg: reg }
    }

    pub fn registrations_snapshot(&self) -> Vec<HandlerRegistration> {
        let g = self.inner.lock().expect("events");
        let mut out = Vec::new();
        for s in &g.middleware {
            out.push(HandlerRegistration {
                id: s.id,
                kind: "middleware",
                filter: s.filter.clone(),
            });
        }
        for s in &g.pipeline {
            out.push(HandlerRegistration {
                id: s.id,
                kind: "pipeline",
                filter: s.filter.clone(),
            });
        }
        for s in &g.parallel {
            out.push(HandlerRegistration {
                id: s.id,
                kind: "parallel",
                filter: s.filter.clone(),
            });
        }
        for s in &g.message {
            out.push(HandlerRegistration {
                id: s.id,
                kind: "message",
                filter: s.filter.clone(),
            });
        }
        out
    }

    pub fn user_sink_count(&self) -> usize {
        self.inner.lock().expect("events").user_sinks.len()
    }

    pub async fn emit_host(&self, event: HostEvent) {
        self.dispatch(BusEvent::Host(event)).await;
    }

    pub async fn emit_ability(&self, event: AbilityEvent) {
        self.dispatch(BusEvent::Ability(event)).await;
    }

    pub async fn emit_user(&self, event: UserEvent) {
        let sinks = self.inner.lock().expect("events").user_sinks.clone();
        for s in sinks {
            s.on_user_event(&event).await;
        }
    }

    async fn dispatch(&self, event: BusEvent) {
        let snap = {
            let g = self.inner.lock().expect("events");
            Snap {
                mw: g
                    .middleware
                    .iter()
                    .filter(|s| s.filter.matches(&event))
                    .map(|s| Arc::clone(&s.handler))
                    .collect(),
                pipeline: g
                    .pipeline
                    .iter()
                    .filter(|s| s.filter.matches(&event))
                    .map(|s| Arc::clone(&s.handler))
                    .collect(),
                parallel: g
                    .parallel
                    .iter()
                    .filter(|s| s.filter.matches(&event))
                    .map(|s| Arc::clone(&s.handler))
                    .collect(),
                message: g
                    .message
                    .iter()
                    .filter(|s| s.filter.matches(&event))
                    .map(|s| Arc::clone(&s.handler))
                    .collect(),
            }
        };
        let _ = run_mw_fut(0, Arc::new(snap), event).await;
    }
}

fn run_mw_fut(
    index: usize,
    snap: Arc<Snap>,
    event: BusEvent,
) -> Pin<Box<dyn Future<Output = Result<BusEvent>> + Send>> {
    Box::pin(async move {
        if index < snap.mw.len() {
            let rest = Arc::clone(&snap);
            let next = EventNext::new(move |ev| run_mw_fut(index + 1, rest, ev));
            snap.mw[index].around(event, next).await
        } else {
            run_inner(snap, event).await
        }
    })
}

async fn run_inner(snap: Arc<Snap>, mut event: BusEvent) -> Result<BusEvent> {
    for p in &snap.pipeline {
        event = p.process(event).await?;
    }
    for s in &snap.message {
        let ev = event.clone();
        let s = Arc::clone(s);
        tokio::spawn(async move {
            s.on_event(&ev).await;
        });
    }
    let mut set = tokio::task::JoinSet::new();
    for s in &snap.parallel {
        let ev = event.clone();
        let s = Arc::clone(s);
        set.spawn(async move {
            s.on_event(&ev).await;
        });
    }
    while set.join_next().await.is_some() {}
    Ok(event)
}

/// 已登记事件处理器摘要。
#[derive(Clone, Debug, Serialize)]
pub struct HandlerRegistration {
    pub id: u64,
    pub kind: &'static str,
    pub filter: EventFilter,
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
