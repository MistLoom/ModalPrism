//! HttpPort 路由扩展点测试：默认 REST 面之上合并应用自有路由。

use std::sync::Arc;

use async_trait::async_trait;
use axum::Json;
use axum::routing::get;
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
        Vec::new()
    }
    fn get_entry(&self, _i: &frame_core::InstanceId, _m: &frame_core::MethodId) -> Option<frame_core::CatalogEntry> {
        None
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn route_extension_serves_app_routes_alongside_default() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener); // 释放端口，避免 serve 重新 bind 时在 Windows 下产生端口争用
    let base = format!("http://{addr}");

    let port = Box::new(HttpPort::new(addr).with_route_extension(|router| {
        router.route("/app/summary", get(|| async { Json(json!({"area": "app"})) }))
    }));
    tokio::spawn(async move { port.serve(Arc::new(Fake)).await.unwrap() });

    // 默认 REST 面仍在
    let health: Value = reqwest::get(format!("{base}/health"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health["status"], "ok");

    // 应用自有路由已合并
    let summary: Value = reqwest::get(format!("{base}/app/summary"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(summary["area"], "app");
}
