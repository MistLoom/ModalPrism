//! 模型调用与通知投递。
//!
//! 内容投递的两种出站形态：模型生成（远端 API、本机端点与命令行程序）与通知
//! 推送（webhook / 飞书）。两者共用同一份出站治理与错误口径，调用方只需给出
//! 声明形状（[`ModelSpec`]、[`Notify`]）。
//!
//! 模型支持三种调用形态：远端 API、本机端点（同一协议）与命令行程序。三者在
//! [`llm::complete`] 之后没有区别，因此投递只需指定一个模型名。

mod error;
mod http;
mod llm;
mod notify;
mod render;
mod spec;

pub use error::{Error, Result};
pub use http::{Reply, request as http_request};
pub use llm::complete as complete_model;
pub use notify::deliver;
pub use render::render as render_template;
pub use spec::{Mode, ModelSpec, Notify, NotifyKind};
