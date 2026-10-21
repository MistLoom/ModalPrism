//! 抽象类型与只读数据入口。
//!
//! 类型实现只返回投影或变更集。写入由调用方把变更集交给 Strata。

mod article;
mod error;
mod hotlist;
mod read;
mod root;

pub use article::Article;
pub use error::{Error, invalid_input};
pub use hotlist::HotList;
pub use root::ReadRoot;

