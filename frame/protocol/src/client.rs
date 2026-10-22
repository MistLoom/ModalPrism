use async_trait::async_trait;

use crate::{ChatCompletionRequest, ChatCompletionResponse, ProtocolError};

/// 聊天补全客户端门面。工作核只依赖此 trait，不绑定具体厂商 SDK。
#[async_trait]
pub trait ChatClient: Send + Sync {
    async fn chat_completion(
        &self,
        request: ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, ProtocolError>;
}
