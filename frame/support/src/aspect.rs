//! 切面契约。由装配织入，不写进 Manifest。实现可在外部工程。

use std::future::Future;
use std::pin::Pin;

use async_trait::async_trait;
use serde_json::Value;

use crate::{AbilityCtx, Result};

pub type BoxInvoke<'a> = Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>>;

#[async_trait]
pub trait Aspect: Send + Sync {
    fn order(&self) -> i32 {
        0
    }

    async fn around(&self, ctx: &dyn AbilityCtx, args: &Value, next: Next<'_>) -> Result<Value>;
}

pub struct Next<'a> {
    inner: Box<dyn FnOnce(Value) -> BoxInvoke<'a> + Send + 'a>,
    default_args: Value,
}

impl<'a> Next<'a> {
    pub fn new<F>(default_args: Value, f: F) -> Self
    where
        F: FnOnce(Value) -> BoxInvoke<'a> + Send + 'a,
    {
        Self {
            inner: Box::new(f),
            default_args,
        }
    }

    pub fn run(self) -> BoxInvoke<'a> {
        (self.inner)(self.default_args)
    }

    pub fn with_args(self, args: Value) -> BoxInvoke<'a> {
        (self.inner)(args)
    }
}
