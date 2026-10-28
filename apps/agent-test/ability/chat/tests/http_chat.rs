//! chat HTTP 适配单元测试。

use ai_protocol::{ChatClient, ChatCompletionRequest, HttpChatClient, HttpChatConfig, Message};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn http_chat_completion_parses_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "hello"
                },
                "finish_reason": "stop"
            }]
        })))
        .mount(&server)
        .await;

    let client = HttpChatClient::new(HttpChatConfig {
        base_url: server.uri(),
        api_key: Some("test".into()),
        path: "/v1/chat/completions".into(),
    });
    let resp = client
        .chat_completion(ChatCompletionRequest::new("m", vec![Message::user("hi")]))
        .await
        .unwrap();
    assert_eq!(
        resp.first_message().unwrap().content_text().as_deref(),
        Some("hello")
    );
}
