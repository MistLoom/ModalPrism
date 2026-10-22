//! 框架门面：Gateway / Port / Expose / 协议控制器。
//!
//! 能力契约在 `ability-support`。本 crate 不导出 Ability / AbilityCtx。

mod controller;
mod expose;
mod gateway;
mod invoke;
mod invoke_mask;
mod merge;
mod port;

pub use ability_support::InvokeMask;
pub use ability_support::{
    AbilityId, AbilityIdError, CatalogEntry, Error, ErrorKind, InstanceId, InstanceIdError,
    Lifecycle, LocalsScope, MethodId, MethodIdError, Result, SourceId, SourceIdError,
    resolve_target,
};
pub use controller::{
    ControllerRegistry, DefaultCliController, DefaultFunController, DefaultGrpcController,
    DefaultHttpController, DefaultMcpController, ProtocolController, ProtocolKind,
};
pub use expose::{
    AbilityExpose, AccessKind, ArtifactKind, CapExpose, ResolvedCap, UnknownPolicy,
    artifact_exe_name,
};
pub use gateway::Gateway;
pub use invoke::{InvokeRequest, InvokeResponse};
pub use invoke_mask::invoke_mask_from_expose;
pub use merge::merge_defaults;
pub use port::Port;
