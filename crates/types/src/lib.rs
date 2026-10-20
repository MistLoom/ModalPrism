//! 抽象类型与只读数据入口。
//!
//! 类型实现只返回投影或变更集。写入由调用方把变更集交给 Strata。

mod article;
mod error;
mod hotlist;
mod read;
mod root;

use serde_json::Value;

pub use article::Article;
pub use error::{Error, invalid_input};
pub use hotlist::HotList;
pub use root::ReadRoot;

impl ReadRoot {
    pub async fn call(&self, method: &str, args: Value) -> Result<Value, Error> {
        let args = read::arguments(args)?;
        match method {
            "strata_read.kinds" => read::kinds(self, &args).await,
            "strata_read.has" => read::has(self, &args).await,
            "strata_read.get" => read::get(self, &args).await,
            "strata_read.slice" => read::slice(self, &args).await,
            "strata_read.resolve" => read::resolve(self, &args).await,
            "strata_read.history" => read::history(self, &args).await,
            "strata_read.lineage" => read::lineage(self, &args).await,
            "strata_read.related" => read::related(self, &args).await,
            "strata_read.query" => read::query(self, &args).await,
            "strata_read.changes" => read::changes(self, &args).await,
            "health.status" => read::health_status(self, &args).await,
            "config_inspect.validate" => read::config_validate(self, &args).await,
            "config_inspect.diff" => read::config_diff(self, &args).await,
            "config_inspect.explain" => read::config_explain(self, &args).await,
            "type_article.project" => article::project_call(self, &args).await,
            "type_article.on_commit" => article::on_commit_call(&args),
            "type_hotlist.project" => hotlist::project_call(self, &args).await,
            "type_hotlist.on_commit" => hotlist::on_commit_call(self, &args).await,
            _ => Err(Error::new("spec 无效")),
        }
    }
}
