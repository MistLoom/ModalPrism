//! `ability!(ctx.algo.echo).run(args)`：运行期 id 用 [`AbilityRef::new`]。

use std::future::Future;

use ability_support::{AbilityCtx, Error, MethodId, Result};
use serde_json::Value;

pub struct AbilityRef<'a> {
    ctx: &'a dyn AbilityCtx,
    id: String,
}

impl<'a> AbilityRef<'a> {
    pub fn new(ctx: &'a dyn AbilityCtx, id: &str) -> Self {
        Self {
            ctx,
            id: id.to_string(),
        }
    }

    pub fn method(self, method: &str) -> MethodRef<'a> {
        MethodRef {
            ctx: self.ctx,
            id: self.id,
            method: method.to_string(),
        }
    }

    pub fn run(self, args: Value) -> impl Future<Output = Result<Value>> + Send + 'a {
        self.method("run").invoke(args)
    }
}

pub struct MethodRef<'a> {
    ctx: &'a dyn AbilityCtx,
    id: String,
    method: String,
}

impl<'a> MethodRef<'a> {
    pub fn invoke(self, args: Value) -> impl Future<Output = Result<Value>> + Send + 'a {
        async move {
            let method =
                MethodId::new(&self.method).map_err(|e| Error::invalid_args(e.to_string()))?;
            self.ctx.call(&self.id, &method, args).await
        }
    }
}
