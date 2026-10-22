//! AI 协议门面：OpenAI 兼容的聊天补全与工具调用类型。
//!
//! 上层工作核 / 处理系统只依赖本 crate 暴露的契约与 [`ChatClient`]；
//! 具体传输（HTTP 等）通过可选 feature 或外部适配器接入，禁止在业务层硬编码厂商细节。

mod chat;
mod client;
mod error;
mod message;
mod tool;

#[cfg(feature = "http")]
mod http;

pub use chat::{
    ChatChoice, ChatCompletionRequest, ChatCompletionResponse, FinishReason, ResponseFormat, Usage,
};
pub use client::ChatClient;
pub use error::ProtocolError;
pub use message::{
    ContentPart, Message, MessageContent, Role, json_content, repair_tool_call_chain,
};
pub use tool::{
    FunctionCall, FunctionDef, ToolCall, ToolCallKind, ToolChoice, ToolChoiceFunction, ToolDef,
    ToolResultMessage,
};

#[cfg(feature = "http")]
pub use http::{HttpChatClient, HttpChatConfig};
