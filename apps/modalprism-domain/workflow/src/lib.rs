//! 内容加工。
//!
//! 一次加工 = 一条工作流，按顺序执行若干步骤（提示词、模板、HTTP 投递），
//! 结果写回文章资源，并可选地推送通知。工作流与模型都是配置目录里的 YAML，
//! 界面和公共 API 共用同一份读写与校验逻辑。
//!
//! 模型支持三种调用形态：远端 API、本机端点（同一协议）与命令行程序。三者在
//! [`llm::complete`] 之后没有区别，因此步骤声明只需要一个 `model` 名。

mod engine;
mod error;
mod http;
mod llm;
mod notify;
mod render;
mod spec;
mod store;

pub use engine::{Engine, Tick};
pub use error::{Error, Result};
pub use http::{Reply, request as http_request};
pub use llm::complete as complete_model;
pub use notify::deliver;
pub use render::render as render_template;
pub use spec::{
    FacetOut, Mode, ModelSpec, Notify, NotifyKind, Step, StepKind, Trigger, WorkflowSpec,
};
pub use store::Store;
