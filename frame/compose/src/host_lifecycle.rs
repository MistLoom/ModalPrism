//! Host 系统事件注册辅助。
//!
//! 引擎在能力 load / unload / invoke 及生命周期相位推进时 emit Host 系统事件；
//! App 在 `AppBuilder::bootstrap` 前登记监听，或在持有 `AbilityHost` 后直接调用 `register`。

use std::sync::Arc;

use ability_core::{AbilityHost, ChannelListener};
use ability_support::{
    EventClass, EventFilter, EventHandler, EventOpts, EventReg, EventSink, host_event,
};

/// 待 bootstrap 应用的 Host 事件登记。
#[derive(Clone)]
pub struct HostRegistration {
    pub filter: EventFilter,
    pub handler: EventHandler,
    pub opts: EventOpts,
}

impl HostRegistration {
    /// 登记到 Host 的 EventBus；丢弃返回的 [`EventReg`] 即卸。
    pub fn register(&self, host: &AbilityHost) -> EventReg {
        host.events()
            .handle(self.filter.clone(), self.handler.clone(), self.opts)
    }
}

/// 构造 Host 事件登记。
pub fn on_host(filter: EventFilter, handler: EventHandler, opts: EventOpts) -> HostRegistration {
    HostRegistration {
        filter,
        handler,
        opts,
    }
}

/// `message` 模式快捷登记。
pub fn on_host_message(filter: EventFilter, sink: Arc<dyn EventSink>) -> HostRegistration {
    on_host(filter, EventHandler::Message(sink), EventOpts::default())
}

/// 批量登记。
pub fn apply_registrations(host: &AbilityHost, regs: &[HostRegistration]) -> Vec<EventReg> {
    regs.iter().map(|r| r.register(host)).collect()
}

/// 拉通道订阅（message 队列）。
pub async fn listen(host: &AbilityHost, filter: EventFilter) -> ChannelListener {
    host.events().listen(filter).await
}

/// 全部 Host 系统事件。
pub fn filter_host_all() -> EventFilter {
    EventFilter {
        class: Some(EventClass::Host),
        ..Default::default()
    }
}

/// 按固定事件名过滤。
pub fn filter_host_named(name: impl Into<String>) -> EventFilter {
    EventFilter {
        class: Some(EventClass::Host),
        name: Some(name.into()),
        ..Default::default()
    }
}

/// `host.load`
pub fn filter_host_load() -> EventFilter {
    filter_host_named(host_event::LOAD)
}

/// `host.unload`
pub fn filter_host_unload() -> EventFilter {
    filter_host_named(host_event::UNLOAD)
}

/// `ability.invoke.before` / `after` / `error`
pub fn filter_host_invoke() -> EventFilter {
    filter_host_all()
}

/// `ability.lifecycle.*`（须在 sink 内按 `event.name()` 前缀过滤）
pub fn filter_host_lifecycle() -> EventFilter {
    filter_host_all()
}
