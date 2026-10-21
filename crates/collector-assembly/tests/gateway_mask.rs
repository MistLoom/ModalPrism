//! 并发 HTTP 运维调用不依赖全局 access_gate。

use std::sync::Arc;

use ability_core::AbilityHost;
use ability_support::{InstanceId, MethodId};
use collector_assembly::{HttpOpsGateway, ops_http_expose, OPS_HTTP_INSTANCES};
use frame_core::{Gateway, InvokeRequest};
use serde_json::json;

#[tokio::test]
async fn concurrent_http_ops_invokes_do_not_block_each_other() {
    let host = Arc::new(AbilityHost::new());
    host.set_entry_app("test");
    let gateway = HttpOpsGateway::new(
        Arc::clone(&host),
        ops_http_expose(),
        OPS_HTTP_INSTANCES,
    );
    assert!(Gateway::catalog(&*gateway).is_empty());

    let g1 = Arc::clone(&gateway);
    let g2 = Arc::clone(&gateway);
    let (a, b) = tokio::join!(
        async move {
            g1.invoke(InvokeRequest::new(
                InstanceId::new("health").expect("id"),
                MethodId::new("status").expect("method"),
                json!({ "source": "source/demo" }),
            ))
            .await
        },
        async move {
            g2.invoke(InvokeRequest::new(
                InstanceId::new("config_inspect").expect("id"),
                MethodId::new("validate").expect("method"),
                json!({}),
            ))
            .await
        }
    );

    assert!(a.is_err() || b.is_err());
}
