//! OpenAI 兼容 HTTP 适配（可选 feature `http`）。

use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};

use crate::{ChatClient, ChatCompletionRequest, ChatCompletionResponse, ProtocolError};

/// HTTP 客户端配置。
#[derive(Clone, Debug)]
pub struct HttpChatConfig {
    /// 例如 `https://api.openai.com/v1` 或兼容网关根路径。
    pub base_url: String,
    pub api_key: Option<String>,
    pub path: String,
}

impl HttpChatConfig {
    pub fn openai_compatible(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: Some(api_key.into()),
            path: "/chat/completions".into(),
        }
    }
}

/// 基于 reqwest 的 OpenAI 兼容客户端。
pub struct HttpChatClient {
    http: reqwest::Client,
    config: HttpChatConfig,
}

impl HttpChatClient {
    pub fn new(config: HttpChatConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            config,
        }
    }

    pub fn with_client(config: HttpChatConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }
}

#[async_trait]
impl ChatClient for HttpChatClient {
    async fn chat_completion(
        &self,
        request: ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, ProtocolError> {
        let url = format!("{}{}", self.config.base_url, self.config.path);
        let mut builder = self
            .http
            .post(&url)
            .header(CONTENT_TYPE, "application/json")
            .json(&request);

        if let Some(ref key) = self.config.api_key {
            builder = builder.header(AUTHORIZATION, format!("Bearer {key}"));
        }

        let response = builder
            .send()
            .await
            .map_err(|e| ProtocolError::Transport(e.to_string()))?;

        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .map_err(|e| ProtocolError::Transport(e.to_string()))?;

        if !(200..300).contains(&status) {
            return Err(ProtocolError::Upstream { status, body });
        }

        serde_json::from_str(&body).map_err(ProtocolError::from)
    }
}
