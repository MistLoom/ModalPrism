//! port-http 鉴权（可选 Bearer）与限流（令牌桶）测试。

use std::sync::Arc;

use ability_support::{RateLimiter, TokenBucket};
use async_trait::async_trait;
use frame_core::{Gateway, InvokeRequest, InvokeResponse, Port};
use frame_port_http::HttpPort;
use serde_json::{Value, json};

struct Fake;
#[async_trait]
impl Gateway for Fake {
    async fn invoke(&self, _r: InvokeRequest) -> Result<InvokeResponse, frame_core::Error> {
        Ok(InvokeResponse::new(json!({"ok": true})))
    }
    fn catalog(&self) -> Vec<frame_core::CatalogEntry> {
        vec![frame_core::CatalogEntry {
            instance: frame_core::InstanceId::new("demo").unwrap(),
            ability_id: frame_core::AbilityId::new("demo-ability").unwrap(),
            method: frame_core::MethodId::new("say").unwrap(),
            description: "d".into(),
            stateful: false,
            phase: frame_core::Lifecycle::Ready,
            input_schema: Value::Null,
            output_schema: None,
        }]
    }
    fn get_entry(
        &self,
        _i: &frame_core::InstanceId,
        _m: &frame_core::MethodId,
    ) -> Option<frame_core::CatalogEntry> {
        None
    }
}

async fn spawn(port: Box<HttpPort>, gateway: Arc<dyn Gateway>) -> String {
    let addr = port.bind;
    tokio::spawn(async move { port.serve(gateway).await.unwrap() });
    format!("http://{addr}")
}

#[tokio::test(flavor = "multi_thread")]
async fn bearer_token_guards_v1_but_not_health() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener); // 仅占位取空闲端口，交给 Port::serve 绑定
    let base = spawn(Box::new(HttpPort::new(addr).with_bearer_token("s3cret")), Arc::new(Fake)).await;

    // /health 豁免
    let health: Value = reqwest::get(format!("{base}/health")).await.unwrap().json().await.unwrap();
    assert_eq!(health["status"], "ok");

    // 无凭证 → 401
    let resp = reqwest::get(format!("{base}/v1/abilities")).await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 错误凭证 → 401
    let resp = reqwest::Client::new()
        .get(format!("{base}/v1/abilities"))
        .header("Authorization", "Bearer wrong")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 正确凭证 → 200
    let resp = reqwest::Client::new()
        .get(format!("{base}/v1/abilities"))
        .header("Authorization", "Bearer s3cret")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn rate_limit_returns_429_with_rate_limited_kind() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let base = format!("http://{addr}");

    // 容量 2，补充速率极低 → 第 3 次调用超限
    let limiter: Arc<dyn RateLimiter> = Arc::new(TokenBucket::new(2.0, f64::EPSILON));
    tokio::spawn(async move {
        Box::new(HttpPort::new(addr).with_rate_limit(limiter))
            .serve(Arc::new(Fake))
            .await
            .unwrap()
    });

    let client = reqwest::Client::new();
    for _ in 0..2 {
        let resp = client
            .post(format!("{base}/v1/abilities/demo.say/invoke"))
            .json(&json!({"args": {}}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
    }
    let resp = client
        .post(format!("{base}/v1/abilities/demo.say/invoke"))
        .json(&json!({"args": {}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["type"], "rate_limited");
}
