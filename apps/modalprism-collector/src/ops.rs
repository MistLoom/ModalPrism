//! 运维门面：把组合根已链接的 Ability 收敛成受白名单约束的 Gateway，并以 HTTP 暴露。

use std::net::SocketAddr;
use std::sync::Arc;

use ability_core::AbilityHost;
use ability_support::{InstanceId, MethodId};
use async_trait::async_trait;
use frame_core::{CatalogEntry, Gateway, InvokeRequest, InvokeResponse};
use frame_port_http::HttpPort;
use tokio::net::TcpListener;

/// 运维面白名单来自 `surface`，此处不重复声明。
fn ops_allowed(instance: &str, method: &str) -> bool {
    surface::exposes(surface::COLLECTOR_OPS, instance, method)
}

pub fn gateway(host: Arc<AbilityHost>) -> Arc<dyn Gateway> {
    Arc::new(OpsGateway { inner: host })
}

pub async fn serve(addr: SocketAddr, host: Arc<AbilityHost>) {
    let app = HttpPort::router(gateway(host)).layer(axum::middleware::map_response(strip_cors));
    let Ok(listener) = TcpListener::bind(addr).await else {
        return;
    };
    let _ = axum::serve(listener, app).await;
}

async fn strip_cors(mut response: axum::response::Response) -> axum::response::Response {
    let names: Vec<_> = response
        .headers()
        .keys()
        .filter(|name| name.as_str().starts_with("access-control-"))
        .cloned()
        .collect();
    for name in names {
        response.headers_mut().remove(name);
    }
    let vary = response
        .headers()
        .get("vary")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    if let Some(vary) = vary {
        let kept: Vec<_> = vary
            .split(',')
            .map(str::trim)
            .filter(|token| !token.is_empty() && !cors_vary(token))
            .collect();
        if kept.is_empty() {
            response.headers_mut().remove("vary");
        } else if let Ok(value) = kept.join(", ").parse() {
            response.headers_mut().insert("vary", value);
        }
    }
    response
}

fn cors_vary(token: &str) -> bool {
    token.eq_ignore_ascii_case("origin")
        || token.eq_ignore_ascii_case("access-control-request-method")
        || token.eq_ignore_ascii_case("access-control-request-headers")
}

struct OpsGateway {
    inner: Arc<AbilityHost>,
}

#[async_trait]
impl Gateway for OpsGateway {
    async fn invoke(&self, request: InvokeRequest) -> ability_support::Result<InvokeResponse> {
        if !ops_allowed(request.instance.as_str(), request.method.as_str()) {
            return Err(ability_support::Error::unavailable("实例未开放"));
        }
        self.inner.invoke(request).await
    }

    fn catalog(&self) -> Vec<CatalogEntry> {
        self.inner
            .catalog()
            .into_iter()
            .filter(|entry| ops_allowed(entry.instance.as_str(), entry.method.as_str()))
            .collect()
    }

    fn get_entry(&self, instance: &InstanceId, method: &MethodId) -> Option<CatalogEntry> {
        if !ops_allowed(instance.as_str(), method.as_str()) {
            return None;
        }
        self.inner.get_entry(instance, method)
    }
}
