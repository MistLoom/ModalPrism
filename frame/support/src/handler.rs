//! 强类型方法糖。无协议 hint。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{AbilityCtx, Error, Result};

type BoxFut = Pin<Box<dyn Future<Output = Result<Value>> + Send + 'static>>;
type BoxCtxFut<'a> = Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>>;

/// 强类型异步方法（无 ctx）。
pub fn typed_method<In, Out, F, Fut>(f: F) -> Arc<dyn Fn(Value) -> BoxFut + Send + Sync>
where
    In: DeserializeOwned + Send + 'static,
    Out: Serialize + Send + 'static,
    F: Fn(In) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Out>> + Send + 'static,
{
    let f = Arc::new(f);
    Arc::new(move |args| {
        let f = Arc::clone(&f);
        Box::pin(async move {
            let input: In =
                serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
            let out = f(input).await?;
            serde_json::to_value(out).map_err(|e| Error::failed(format!("响应序列化失败: {e}")))
        })
    })
}

/// 带 ctx 的强类型方法。
pub fn typed_method_with_ctx<In, Out, F>(
    f: F,
) -> Arc<dyn for<'a> Fn(&'a dyn AbilityCtx, Value) -> BoxCtxFut<'a> + Send + Sync>
where
    In: DeserializeOwned + Send + 'static,
    Out: Serialize + Send + 'static,
    F: for<'a> Fn(&'a dyn AbilityCtx, In) -> Pin<Box<dyn Future<Output = Result<Out>> + Send + 'a>>
        + Send
        + Sync
        + 'static,
{
    let f = Arc::new(f);
    Arc::new(move |ctx, args| match serde_json::from_value::<In>(args) {
        Err(e) => Box::pin(async move { Err(Error::invalid_args(e.to_string())) }),
        Ok(input) => {
            let fut = f(ctx, input);
            Box::pin(async move {
                let out = fut.await?;
                serde_json::to_value(out).map_err(|e| Error::failed(format!("响应序列化失败: {e}")))
            })
        }
    })
}
