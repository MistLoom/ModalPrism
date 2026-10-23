//! 抽象类型与只读数据入口。
//!
//! 类型实现只返回投影或变更集。写入由调用方把变更集交给 Strata。
//! 只读投影（`strata_read`、`health.status`、`config_inspect` 与类型投影）只有
//! 一份实现，见 [`read::call`]；两个进程各自提供 [`ReadCtx`] 实现即可复用。

mod article;
mod ctx;
mod error;
mod hotlist;
pub mod read;
mod root;

use serde_json::Value;

pub use article::Article;
pub use ctx::ReadCtx;
pub use error::{Error, invalid_input};
pub use hotlist::HotList;
pub use root::ReadRoot;

impl ReadRoot {
    /// 以自身为只读上下文调用只读方法。
    pub async fn call(&self, method: &str, args: Value) -> Result<Value, Error> {
        read::call(self, method, args).await
    }
}
