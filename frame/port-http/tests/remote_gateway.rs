//! RemoteHttpGateway 回环测试：进程内假 Gateway 起 HTTP 端口，远端客户端连回调用。

use std::sync::Arc;

use async_trait::async_trait;
use frame_core::{CatalogEntry, Gateway, InstanceId, InvokeRequest, InvokeResponse, MethodId};
use frame_port_http::{HttpPort, RemoteHttpGateway};
use serde_json::{Value, json};

struct FakeGateway {
    fail_invoke: bool,
}

#[async_trait]
impl Gateway for FakeGateway {
    async fn invoke(&self, request: InvokeRequest) -> Result<InvokeResponse, frame_core::Error> {
        if self.fail_invoke {
            return Err(frame_core::Error::not_found("demo.echo.say"));
        }
        let scope = request.locals_scope.as_ref().map(|s| s.to_string());
        Ok(InvokeResponse::new(json!({
            "echo": request.args,
            "scope": scope,
            "mask": request.invoke_mask,
        }))
        .with_locals(request.locals_scope.unwrap_or_else(frame_core::LocalsScope::new)))
    }

    fn catalog(&self) -> Vec<CatalogEntry> {
        vec![CatalogEntry {
            instance: InstanceId::new("demo").unwrap(),
            ability_id: frame_core::AbilityId::new("demo-ability").unwrap(),
            method: MethodId::new("echo.say").unwrap(),
            description: "回声".into(),
            stateful: false,
            phase: frame_core::Lifecycle::Ready,
            input_schema: Value::Null,
            output_schema: None,
        }]
    }

    fn get_entry(&self, instance: &InstanceId, method: &MethodId) -> Option<CatalogEntry> {
        if instance.as_str() == "demo" && method.as_str() == "echo.say" {
            self.catalog().into_iter().next()
        } else {
            None
        }
    }
}

async fn spawn_server(gateway: Arc<dyn Gateway>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = HttpPort::router(gateway);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test(flavor = "multi_thread")]
async fn remote_invoke_roundtrip() {
    let base = spawn_server(Arc::new(FakeGateway { fail_invoke: false })).await;
    let remote = RemoteHttpGateway::connect_async(&base).await.unwrap();

    // 目录经 REST 还原
    assert_eq!(remote.catalog().len(), 1);
    let entry = remote
        .get_entry(&InstanceId::new("demo").unwrap(), &MethodId::new("echo.say").unwrap())
        .expect("条目应可查");
    assert_eq!(entry.description, "回声");

    // invoke 透传 args / locals / mask
    let request = InvokeRequest::new(
        InstanceId::new("demo").unwrap(),
        MethodId::new("echo.say").unwrap(),
        json!({"text": "hi"}),
    )
    .with_invoke_mask(frame_core::InvokeMask {
        allow: Some(["demo".to_string()].into_iter().collect()),
        deny: Default::default(),
    });
    let response = remote.invoke(request).await.unwrap();
    assert_eq!(response.data["echo"]["text"], "hi");
    // locals_scope 有回传（服务端生成新作用域）
    assert!(response.locals_scope.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn remote_error_kind_restored() {
    let base = spawn_server(Arc::new(FakeGateway { fail_invoke: true })).await;
    let remote = RemoteHttpGateway::connect_async(&base).await.unwrap();

    let request = InvokeRequest::new(
        InstanceId::new("demo").unwrap(),
        MethodId::new("echo.say").unwrap(),
        json!({}),
    );
    let err = remote.invoke(request).await.unwrap_err();
    // 分类码还原，不降级为泛化 upstream
    assert_eq!(err.kind(), frame_core::ErrorKind::NotFound);
    assert!(err.to_string().contains("demo.echo.say"));
}

#[tokio::test(flavor = "multi_thread")]
async fn remote_unreachable_is_unavailable() {
    // 不可达端口：连接型 Gateway 报 Unavailable，可被上层识别重试
    let remote = RemoteHttpGateway::connect("http://127.0.0.1:1");
    let request = InvokeRequest::new(
        InstanceId::new("demo").unwrap(),
        MethodId::new("echo.say").unwrap(),
        json!({}),
    );
    let err = remote.invoke(request).await.unwrap_err();
    assert_eq!(err.kind(), frame_core::ErrorKind::Unavailable);
}
