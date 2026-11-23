//! 方法逻辑。声明与协议投影不在此处。
//!
//! `plan.*` 动词与 logic 文件一一对应（治理测试按 `method.replace('.', "_")`
//! 核对文件名 `plan_ls.rs` 等）。

pub mod catalog;
pub mod fetch;
pub mod ls;
pub mod once;
pub mod parse;
pub mod probe;
pub mod read;
pub mod save;
pub mod search;

#[path = "plan_get.rs"]
pub mod plan_get;
#[path = "plan_ls.rs"]
pub mod plan_ls;
#[path = "plan_new.rs"]
pub mod plan_new;
#[path = "plan_pause.rs"]
pub mod plan_pause;
#[path = "plan_resume.rs"]
pub mod plan_resume;
#[path = "plan_run.rs"]
pub mod plan_run;
#[path = "plan_set.rs"]
pub mod plan_set;
#[path = "strategy_ls.rs"]
pub mod strategy_ls;
#[path = "strategy_get.rs"]
pub mod strategy_get;
#[path = "strategy_new.rs"]
pub mod strategy_new;
#[path = "strategy_set.rs"]
pub mod strategy_set;
#[path = "strategy_remove.rs"]
pub mod strategy_remove;
