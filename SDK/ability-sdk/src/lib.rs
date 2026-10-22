//! Ability 作者 SDK。独立外部工程，只依赖 `ability-support`，禁止依赖 `frame-core` / port。
//!
//! 跨能力调用：对方 crate 用 [`client`] 声明方法签名；本侧 `use FooExt` 后
//! `ctx.foo().method(typed_args)`（声明合并）。动态 id 仍可用 `call!` / [`AbilityRef`]。

mod cli;
mod guest;
mod invoke;

pub use ability_support::{
    Ability, AbilityCtx, AbilityId, AbilityManifest, AbilityProvider, AbilitySpec, Aspect,
    BusEvent, CatalogEntry, ChainStatus, CtxFnAbility, Dependency, DiscoveryPayload, Effect, Error,
    ErrorKind, EventClass, EventDecl, EventDir, EventFilter, EventHandler, EventListener,
    EventMiddleware, EventMode, EventNext, EventOpts, EventPipeline, EventReg, EventSink,
    FactoryProvider, FeedbackProcessor, FeedbackRecord, FeedbackScope, FnAbility, InstanceId,
    Lifecycle, LoadSpec, MethodId, MethodMeta, Next, ProtocolKind, Result, SourceId,
    StaticProvider, TagFilter, Undo, box_fut, box_life_fut, host_event, sdk_client as client,
};
pub use cli::build_standalone;
pub use guest::{serve, serve_stdio};
pub use invoke::{AbilityRef, MethodRef};

pub mod prelude {
    pub use super::invoke::AbilityRef;
    pub use super::{
        AbilityCtx, AbilityProvider, AbilitySpec, Aspect, Error, Lifecycle, Result, ability, call,
        client, emit, feedback, handler, life, method_id,
    };
    pub use serde_json::{Value, json};
}

#[doc(hidden)]
#[macro_export]
macro_rules! __id {
    ($last:ident) => {
        stringify!($last)
    };
    ($head:ident . $($tail:ident).+) => {
        concat!(stringify!($head), ".", $crate::__id!($($tail).+))
    };
}

#[macro_export]
macro_rules! method_id {
    ($($part:ident).+) => {
        $crate::MethodId::new($crate::__id!($($part).+)).expect("方法标识")
    };
}

#[macro_export]
macro_rules! call {
    ($ctx:ident . $($ability:ident).+ :: $($method:ident).+ ($args:expr)) => {
        $ctx.call(
            $crate::__id!($($ability).+),
            &$crate::method_id!($($method).+),
            $args,
        )
    };
}

#[macro_export]
macro_rules! ability {
    ($ctx:ident . $($id:ident).+) => {
        $crate::AbilityRef::new($ctx, $crate::__id!($($id).+))
    };
}

#[macro_export]
macro_rules! emit {
    ($ctx:ident . $($name:ident).+ ($payload:expr)) => {
        $ctx.emit($crate::__id!($($name).+), $payload)
    };
}

#[doc(hidden)]
pub use ability_support::feedback::assert_tag as __assert_tag;
#[doc(hidden)]
pub use ability_support::feedback::submit as __feedback_submit;

/// 投递反馈。路径走本 crate，外部工程不必直接依赖 ability-support。
#[macro_export]
macro_rules! feedback {
    ([$($tag:literal),* $(,)?], $($t:tt)+) => {{
        $(
            const _: () = $crate::__assert_tag($tag);
        )*
        $crate::__feedback_submit(
            env!("CARGO_PKG_NAME"),
            module_path!(),
            &[$($tag),*],
            format!($($t)+),
        );
    }};
    ($tags:expr, $($t:tt)+) => {{
        let __tags = &$tags;
        let __tags: &[&str] = ::std::convert::AsRef::as_ref(__tags);
        $crate::__feedback_submit(
            env!("CARGO_PKG_NAME"),
            module_path!(),
            __tags,
            format!($($t)+),
        );
    }};
}

#[macro_export]
macro_rules! handler {
    (|$ctx:ident, $args:ident| $body:expr) => {
        move |$ctx: &dyn $crate::AbilityCtx, $args: ::serde_json::Value| {
            $crate::box_fut(async move { $body })
        }
    };
}

#[macro_export]
macro_rules! life {
    (|$phase:ident, $ctx:ident| $body:expr) => {
        move |$phase: $crate::Lifecycle, $ctx: &dyn $crate::AbilityCtx| {
            $crate::box_life_fut(async move { $body })
        }
    };
}
