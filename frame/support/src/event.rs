//! 三类事件分型。禁止混写。
//!
//! Host / 能力事件的处理合成：middleware（洋葱，默认）包裹 Pipeline，
//! 然后分叉 parallel（汇合）与 message（投递不等待）。用户事件仍走通道。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::manifest::{EventDecl, EventDir};
use crate::{AbilityId, InstanceId, Lifecycle, MethodMeta, Result};

/// 事件大类。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventClass {
    Host,
    Ability,
    User,
}

/// Host 系统事件名（规范固定）。
pub mod host_event {
    pub const LOAD: &str = "host.load";
    pub const UNLOAD: &str = "host.unload";
    pub const LIFECYCLE_PREFIX: &str = "ability.lifecycle.";
    pub const INVOKE_BEFORE: &str = "ability.invoke.before";
    pub const INVOKE_AFTER: &str = "ability.invoke.after";
    pub const INVOKE_ERROR: &str = "ability.invoke.error";

    pub fn lifecycle(phase: &str) -> String {
        format!("{LIFECYCLE_PREFIX}{phase}")
    }
}

/// 已装实例摘要。`AbilityCtx::catalog` 返回此结构。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiscoveryPayload {
    pub instance: InstanceId,
    pub ability_id: AbilityId,
    pub phase: Lifecycle,
    pub stateful: bool,
    pub methods: Vec<MethodMeta>,
    /// 对外开放的可注册事件（`dir: emit`）。
    pub events: Vec<EventDecl>,
}

impl DiscoveryPayload {
    pub fn from_manifest(
        instance: InstanceId,
        phase: Lifecycle,
        manifest: &crate::AbilityManifest,
    ) -> Self {
        Self {
            instance,
            ability_id: manifest.id.clone(),
            phase,
            stateful: manifest.stateful,
            methods: manifest.methods.clone(),
            events: manifest
                .events
                .iter()
                .filter(|e| e.dir == EventDir::Emit)
                .cloned()
                .collect(),
        }
    }
}

/// Host 系统事件。仅 ability-core emit。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HostEvent {
    pub name: String,
    pub at: u64,
    pub discovery: Option<DiscoveryPayload>,
    #[serde(default)]
    pub detail: Value,
}

/// 能力事件。仅该 Ability 按 Manifest emit。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AbilityEvent {
    pub instance: InstanceId,
    pub name: String,
    pub payload: Value,
    pub at: u64,
}

/// 用户事件。仅调用方写入。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserEvent {
    pub name: String,
    pub payload: Value,
    pub at: u64,
}

impl UserEvent {
    pub fn new(name: impl Into<String>, payload: Value) -> Self {
        Self {
            name: name.into(),
            payload,
            at: 0,
        }
    }
}

/// 总线上的一条事件。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "class", rename_all = "snake_case")]
pub enum BusEvent {
    Host(HostEvent),
    Ability(AbilityEvent),
    User(UserEvent),
}

impl BusEvent {
    pub fn class(&self) -> EventClass {
        match self {
            Self::Host(_) => EventClass::Host,
            Self::Ability(_) => EventClass::Ability,
            Self::User(_) => EventClass::User,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Host(e) => &e.name,
            Self::Ability(e) => &e.name,
            Self::User(e) => &e.name,
        }
    }

    /// Host 用 `detail`，能力 / 用户用 `payload`。
    pub fn payload(&self) -> &Value {
        match self {
            Self::Host(e) => &e.detail,
            Self::Ability(e) => &e.payload,
            Self::User(e) => &e.payload,
        }
    }

    pub fn payload_mut(&mut self) -> &mut Value {
        match self {
            Self::Host(e) => &mut e.detail,
            Self::Ability(e) => &mut e.payload,
            Self::User(e) => &mut e.payload,
        }
    }

    pub fn instance(&self) -> Option<&InstanceId> {
        match self {
            Self::Host(e) => e.discovery.as_ref().map(|d| &d.instance),
            Self::Ability(e) => Some(&e.instance),
            Self::User(_) => None,
        }
    }
}

/// 运行时订阅过滤。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EventFilter {
    pub class: Option<EventClass>,
    pub instance: Option<InstanceId>,
    pub name: Option<String>,
}

impl EventFilter {
    pub fn matches(&self, ev: &BusEvent) -> bool {
        if let Some(c) = self.class {
            if ev.class() != c {
                return false;
            }
        }
        if let Some(ref n) = self.name {
            if ev.name() != n {
                return false;
            }
        }
        if let Some(ref id) = self.instance {
            match ev.instance() {
                Some(i) if i == id => {}
                _ => return false,
            }
        }
        true
    }
}

/// 事件拉取。实现在 ability-core。
#[async_trait::async_trait]
pub trait EventListener: Send {
    async fn next(&mut self) -> Option<BusEvent>;
}

/// Host / 能力事件观察者。用于 `message` / `parallel`。
#[async_trait::async_trait]
pub trait EventSink: Send + Sync {
    async fn on_event(&self, event: &BusEvent);
}

/// 用户事件观察者。与 [`EventSink`] 分型。
#[async_trait::async_trait]
pub trait UserEventSink: Send + Sync {
    async fn on_user_event(&self, event: &UserEvent);
}

/// 协作模式。登记时显式选择；`subscribe` / `listen` 为 [`EventMode::Message`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventMode {
    Message,
    Parallel,
    Pipeline,
    Middleware,
}

/// 登记选项。默认追加到该模式队列尾；`prepend` 插入队头（优先执行）。
#[derive(Clone, Copy, Debug, Default)]
pub struct EventOpts {
    pub prepend: bool,
}

impl EventOpts {
    pub fn prepend() -> Self {
        Self { prepend: true }
    }
}

pub type BoxEvent = Pin<Box<dyn Future<Output = Result<BusEvent>> + Send>>;

/// 洋葱下一环。不调用即短路后续监听者；不抹除已触发记录。
pub struct EventNext {
    inner: Box<dyn FnOnce(BusEvent) -> BoxEvent + Send>,
}

impl EventNext {
    pub fn new<F>(f: F) -> Self
    where
        F: FnOnce(BusEvent) -> BoxEvent + Send + 'static,
    {
        Self { inner: Box::new(f) }
    }

    pub fn run(self, event: BusEvent) -> BoxEvent {
        (self.inner)(event)
    }
}

/// 默认协作：可拦截或 `next` 放行。
#[async_trait::async_trait]
pub trait EventMiddleware: Send + Sync {
    async fn around(&self, event: BusEvent, next: EventNext) -> Result<BusEvent>;
}

/// 按登记序变换载荷（清洗 / 校验 / 格式转换）。
#[async_trait::async_trait]
pub trait EventPipeline: Send + Sync {
    async fn process(&self, event: BusEvent) -> Result<BusEvent>;
}

/// 一次登记。丢弃即卸；同一处理者多次登记累加。
pub enum EventHandler {
    Middleware(Arc<dyn EventMiddleware>),
    Pipeline(Arc<dyn EventPipeline>),
    Parallel(Arc<dyn EventSink>),
    Message(Arc<dyn EventSink>),
}

impl EventHandler {
    pub fn mode(&self) -> EventMode {
        match self {
            Self::Middleware(_) => EventMode::Middleware,
            Self::Pipeline(_) => EventMode::Pipeline,
            Self::Parallel(_) => EventMode::Parallel,
            Self::Message(_) => EventMode::Message,
        }
    }
}

impl Clone for EventHandler {
    fn clone(&self) -> Self {
        match self {
            Self::Middleware(h) => Self::Middleware(Arc::clone(h)),
            Self::Pipeline(h) => Self::Pipeline(Arc::clone(h)),
            Self::Parallel(h) => Self::Parallel(Arc::clone(h)),
            Self::Message(h) => Self::Message(Arc::clone(h)),
        }
    }
}

/// 卸登记。实现在引擎 / 客端。
pub trait EventUnreg: Send + Sync {
    fn unregister(&self, id: u64);
}

/// 丢弃返回值即卸该次登记。
pub struct EventReg {
    id: u64,
    unreg: Option<Arc<dyn EventUnreg>>,
}

impl EventReg {
    pub fn new(id: u64, unreg: Arc<dyn EventUnreg>) -> Self {
        Self {
            id,
            unreg: Some(unreg),
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }
}

impl Drop for EventReg {
    fn drop(&mut self) {
        if let Some(u) = self.unreg.take() {
            u.unregister(self.id);
        }
    }
}
