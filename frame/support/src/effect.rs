//! 可逆副作用：apply 返回 Undo，scope 退出时 LIFO 执行。

use std::future::Future;
use std::pin::Pin;

use async_trait::async_trait;

use crate::{AbilityCtx, Result};

type UndoFut = Pin<Box<dyn Future<Output = Result<()>> + Send>>;
type UndoFn = Box<dyn FnOnce() -> UndoFut + Send>;

/// 副作用。在当前 scope 上 apply，登记 Undo。
#[async_trait]
pub trait Effect: Send + Sync {
    async fn apply(&self, ctx: &dyn AbilityCtx) -> Result<Undo>;
}

/// 撤销闭包。失败不得吞掉原始错误，由引擎挂到 cause。
pub struct Undo {
    inner: UndoFn,
}

impl Undo {
    pub fn new<F, Fut>(f: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        Self {
            inner: Box::new(move || Box::pin(f())),
        }
    }

    pub fn noop() -> Self {
        Self::new(|| async { Ok(()) })
    }

    pub async fn run(self) -> Result<()> {
        (self.inner)().await
    }
}
