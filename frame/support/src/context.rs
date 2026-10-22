//! 能力执行上下文。能力只通过本 trait 接触引擎。

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::{
    Aspect, ChainStatus, DiscoveryPayload, Effect, EventFilter, EventHandler, EventListener,
    EventOpts, EventReg, InstanceId, LoadSpec, MethodId, ProtocolKind, Result,
};

/// 能力执行上下文。
#[async_trait]
pub trait AbilityCtx: Send + Sync {
    /// 供 Start 后的订阅任务持有。独立 locals / undos；`call` 仍走 Gateway。
    fn share(&self) -> Arc<dyn AbilityCtx>;

    /// 同一 locals / undos / 绑定的可持有句柄，生命周期可超出当前栈帧。
    /// 远程 Ability 在 invoke / lifecycle 期间回呼时使用。
    fn retain(&self) -> Arc<dyn AbilityCtx>;

    fn instance(&self) -> &InstanceId;

    fn method(&self) -> Option<&MethodId>;

    fn chain_id(&self) -> &str;

    fn span(&self) -> u64;

    /// 在途链路。`chain_id` 为空则查当前调用链；已结束则 `None`。
    fn chain_status(&self, chain_id: Option<&str>) -> Option<ChainStatus>;

    /// 管道内组合。id 为 depends 键（alias 或能力 id）。
    async fn call(&self, id_or_alias: &str, method: &MethodId, args: Value) -> Result<Value>;

    fn dep(&self, id_or_alias: &str) -> Result<InstanceId>;

    /// 已装实例摘要（id、phase、methods、events）。方法面由端口对 Gateway 取。
    fn catalog(&self) -> Vec<DiscoveryPayload>;

    /// 按协议种类裁剪后的 catalog 视图；未登记协议支持时该实例不可见。
    fn with_protocol_filter(&self, kind: ProtocolKind) -> Arc<dyn AbilityCtx> {
        let _ = kind;
        self.retain()
    }

    /// 运行时订 Host / 能力事件（`message` 拉通道）。Stop 时自动卸。
    async fn subscribe(&self, filter: EventFilter) -> Result<Box<dyn EventListener + Send>>;

    /// 登记 Host / 能力事件处理。默认模式见 [`EventHandler`]；`prepend` 优先。
    /// 重复登记累加。丢弃 [`EventReg`] 即卸；Stop 时卸仍持有的登记。
    async fn handle(
        &self,
        filter: EventFilter,
        handler: EventHandler,
        opts: EventOpts,
    ) -> Result<EventReg>;

    async fn load(&self, spec: LoadSpec) -> Result<InstanceId>;

    async fn unload(&self, instance: &InstanceId) -> Result<()>;

    async fn local_get(&self, key: &str) -> Result<Option<Value>>;

    async fn local_set(&self, key: &str, value: Value) -> Result<()>;

    async fn push_effect(&self, effect: Box<dyn Effect>) -> Result<()>;

    /// 仅限本 Manifest 声明为 emit 的能力事件。
    async fn emit(&self, name: &str, payload: Value) -> Result<()>;

    /// 向宿主登记切面（通常在 `Lifecycle::Started` 内调用）。
    fn register_aspect(&self, aspect: Arc<dyn Aspect>) -> Result<()>;
}
