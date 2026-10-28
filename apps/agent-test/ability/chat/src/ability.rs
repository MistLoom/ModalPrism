//! Ability `chat`：封装 `HttpChatClient`。

use std::sync::Arc;

use ability_support::{
    Ability, AbilityCtx, AbilityManifest, AbilityProvider, AbilitySpec, Error, MethodId, Result,
};
use ai_protocol::{
    ChatClient, ChatCompletionRequest, HttpChatClient, HttpChatConfig, Message, ToolChoice,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::generated::manifest;

pub fn provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::from_manifest(manifest::manifest()).into_factory(|config| async move {
        let base_url = config
            .get("base_url")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::invalid_args("chat 配置缺少 base_url"))?
            .trim_end_matches('/')
            .to_string();
        let api_key = config
            .get("api_key")
            .and_then(Value::as_str)
            .map(str::to_string);
        let model = config
            .get("model")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::invalid_args("chat 配置缺少 model"))?
            .to_string();
        let path = if base_url.ends_with("/v1") {
            "/chat/completions".into()
        } else {
            "/v1/chat/completions".into()
        };
        let client = HttpChatClient::new(HttpChatConfig {
            base_url,
            api_key,
            path,
        });
        Ok(Arc::new(ChatAbility {
            manifest: manifest::manifest(),
            client: Arc::new(client),
            model,
        }) as Arc<dyn Ability>)
    })
}

struct ChatAbility {
    manifest: AbilityManifest,
    client: Arc<HttpChatClient>,
    model: String,
}

#[derive(Deserialize)]
struct CompletionArgs {
    messages: Vec<Message>,
    #[serde(default)]
    tools: Option<Vec<ai_protocol::ToolDef>>,
    #[serde(default)]
    tool_choice: Option<String>,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    model: Option<String>,
}

fn map_protocol_err(e: ai_protocol::ProtocolError) -> Error {
    Error::failed(e.to_string())
}

#[async_trait]
impl Ability for ChatAbility {
    fn manifest(&self) -> &AbilityManifest {
        &self.manifest
    }

    async fn invoke(&self, method: &MethodId, _ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
        match method.as_str() {
            "completion" => {
                let a: CompletionArgs =
                    serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
                let model = a.model.unwrap_or_else(|| self.model.clone());
                let mut req = ChatCompletionRequest::new(model, a.messages);
                if let Some(tools) = a.tools {
                    req = req.with_tools(tools);
                }
                if let Some(tc) = a.tool_choice {
                    req.tool_choice = Some(parse_tool_choice(&tc));
                }
                if let Some(t) = a.temperature {
                    req = req.with_temperature(t);
                }
                if let Some(n) = a.max_tokens {
                    req.max_tokens = Some(n);
                }
                let resp = self
                    .client
                    .chat_completion(req)
                    .await
                    .map_err(map_protocol_err)?;
                serde_json::to_value(resp).map_err(|e| Error::failed(e.to_string()))
            }
            other => Err(Error::not_found(other)),
        }
    }
}

fn parse_tool_choice(raw: &str) -> ToolChoice {
    match raw.trim().to_ascii_lowercase().as_str() {
        "none" => ToolChoice::none(),
        "required" => ToolChoice::required(),
        "auto" | "" => ToolChoice::auto(),
        name => ToolChoice::function(name),
    }
}
