//! Ability 核心：运行引擎。
//! 能力契约在 ability-support；Gateway / Expose 在 frame-core；组合根在 frame-compose。

mod chain;
mod context;
mod event_bus;
mod guest_source;
mod host;
mod proc;
mod scope;

pub use event_bus::{ChannelListener, EventBus};
pub use guest_source::GuestSourceProvider;
pub use host::{AbilityGrant, AbilityHost};
pub use proc::{ProcProvider, ProcSupervisor, connect};
