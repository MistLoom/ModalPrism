//! 函数式 Ability：闭包快速实现，供测试与简单能力。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::{Ability, AbilityCtx, AbilityManifest, AbilityProvider, Error, MethodId, Result};

type BoxFut = Pin<Box<dyn Future<Output = Result<Value>> + Send + 'static>>;
type BoxCtxFut<'a> = Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>>;

/// 无 ctx 的异步方法 Ability（单方法）。
pub struct FnAbility {
    manifest: AbilityManifest,
    f: Arc<dyn Fn(Value) -> BoxFut + Send + Sync>,
}

impl FnAbility {
    pub fn new<F, Fut>(manifest: AbilityManifest, f: F) -> Self
    where
        F: Fn(Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value>> + Send + 'static,
    {
        Self {
            manifest,
            f: Arc::new(move |args| Box::pin(f(args))),
        }
    }
}

#[async_trait]
impl Ability for FnAbility {
    fn manifest(&self) -> &AbilityManifest {
        &self.manifest
    }

    async fn invoke(&self, method: &MethodId, _ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
        if self.manifest.method(method).is_none() {
            return Err(Error::not_found(method));
        }
        (self.f)(args).await
    }
}

/// 带 ctx 的异步方法 Ability。
pub struct CtxFnAbility {
    manifest: AbilityManifest,
    f: Arc<dyn for<'a> Fn(&'a dyn AbilityCtx, Value) -> BoxCtxFut<'a> + Send + Sync>,
}

impl CtxFnAbility {
    pub fn new<F>(manifest: AbilityManifest, f: F) -> Self
    where
        F: for<'a> Fn(&'a dyn AbilityCtx, Value) -> BoxCtxFut<'a> + Send + Sync + 'static,
    {
        Self {
            manifest,
            f: Arc::new(f),
        }
    }
}

#[async_trait]
impl Ability for CtxFnAbility {
    fn manifest(&self) -> &AbilityManifest {
        &self.manifest
    }

    async fn invoke(&self, method: &MethodId, ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
        if self.manifest.method(method).is_none() {
            return Err(Error::not_found(method));
        }
        (self.f)(ctx, args).await
    }
}

/// 把已构造的 Ability 作成 Provider。
pub struct StaticProvider {
    ability: Arc<dyn Ability>,
}

impl StaticProvider {
    pub fn new(ability: Arc<dyn Ability>) -> Self {
        Self { ability }
    }
}

#[async_trait]
impl AbilityProvider for StaticProvider {
    fn manifest(&self) -> AbilityManifest {
        self.ability.manifest().clone()
    }

    async fn create(&self, _config: Value) -> Result<Arc<dyn Ability>> {
        Ok(Arc::clone(&self.ability))
    }
}
