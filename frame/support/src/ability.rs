//! Ability 单元与 Provider。能力只依赖 ability-support，禁止依赖 frame-core / port / ability-core。

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::{AbilityCtx, AbilityManifest, Lifecycle, MethodId, Result};

/// 能力实现。
#[async_trait]
pub trait Ability: Send + Sync {
    fn manifest(&self) -> &AbilityManifest;

    async fn on_lifecycle(&self, phase: Lifecycle, ctx: &dyn AbilityCtx) -> Result<()> {
        let _ = (phase, ctx);
        Ok(())
    }

    async fn invoke(&self, method: &MethodId, ctx: &dyn AbilityCtx, args: Value) -> Result<Value>;
}

/// 装载源。Host.install 登记，load 时 create。
///
/// 子进程 / WASM 等来源在 `create` 里握手；因此 `create` 为异步。
#[async_trait]
pub trait AbilityProvider: Send + Sync {
    fn manifest(&self) -> AbilityManifest;

    async fn create(&self, config: Value) -> Result<Arc<dyn Ability>>;
}
