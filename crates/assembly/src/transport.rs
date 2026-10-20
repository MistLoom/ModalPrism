//! 出站传输。地址由网关在 SSRF 检查之后给出。

use std::net::IpAddr;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use accessors::EgressGateway;
use async_trait::async_trait;
use egress::{Policy, Resolver, TcpDial, Transport};
use paid::PaidTransport;
use serde_json::Value;
use strata_model::ErrorClass;
use url::Url;

use accessors::{AccessError, HttpRequest, HttpResponse, PaidCall, PaidResponse};

use crate::queue::Hot;

pub struct HttpTransport;

pub struct SystemResolver;

pub struct PaidBridge {
    pub egress: Arc<EgressGw>,
}

pub type EgressGw = egress::Gateway<Hot, HttpTransport, SystemResolver, TcpDial>;
pub type PaidGw = paid::Gateway<Hot, PaidBridge>;

fn permanent(message: &str) -> AccessError {
    AccessError::new(ErrorClass::Permanent, message)
}

fn transient(message: &str, source: impl std::error::Error + Send + Sync + 'static) -> AccessError {
    AccessError::with_source(ErrorClass::Transient, message, source)
}

#[async_trait]
impl Transport for HttpTransport {
    async fn send(
        &self,
        request: HttpRequest,
        address: SocketAddr,
    ) -> Result<HttpResponse, AccessError> {
        let url = Url::parse(&request.url).map_err(|_| permanent("scheme"))?;
        let host = url.host_str().ok_or_else(|| permanent("scheme"))?;
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| permanent("scheme"))?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .resolve(host, address)
            .build()
            .map_err(|source| transient("transport", source))?;
        let mut builder = client.request(method, request.url);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        if !request.body.is_empty() {
            builder = builder.body(request.body.to_vec());
        }
        let response = builder
            .send()
            .await
            .map_err(|source| transient("transport", source))?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|text| (name.as_str().to_string(), text.to_string()))
            })
            .collect();
        let final_url = response.url().to_string();
        let body = response
            .bytes()
            .await
            .map_err(|source| transient("transport", source))?;
        Ok(HttpResponse {
            status,
            headers,
            body,
            final_url,
        })
    }
}

#[async_trait]
impl Resolver for SystemResolver {
    async fn lookup(&self, host: &str) -> Result<Vec<IpAddr>, AccessError> {
        match tokio::net::lookup_host((host, 80)).await {
            Ok(items) => Ok(items.map(|addr| addr.ip()).collect()),
            Err(source) => Err(transient("dns", source)),
        }
    }
}

#[async_trait]
impl PaidTransport for PaidBridge {
    async fn send(&self, call: PaidCall) -> Result<PaidResponse, AccessError> {
        let Ok(mut url) = Url::parse(&call.endpoint) else {
            return Err(permanent("scheme"));
        };
        if url.scheme() != "http" && url.scheme() != "https" {
            return Err(permanent("scheme"));
        }
        let method = call
            .params
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("GET")
            .to_string();
        if let Some(object) = call.params.as_object() {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in object {
                if key == "method" {
                    continue;
                }
                let text = match value {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                };
                pairs.append_pair(key, &text);
            }
        }
        let response = EgressGateway::request(
            self.egress.as_ref(),
            HttpRequest {
                method,
                url: url.to_string(),
                headers: Vec::new(),
                body: call.body,
            },
        )
        .await?;
        Ok(PaidResponse {
            status: response.status,
            headers: response.headers,
            body: response.body,
            receipt_id: None,
        })
    }
}

pub fn policy_of(system: &Value) -> Policy {
    let mut policy = Policy::library("ModalPrism");
    if system
        .pointer("/policy/egress/private_networks")
        .and_then(Value::as_str)
        == Some("allow")
    {
        policy.private_networks = egress::PrivateNetworks::Allow;
    }
    if let Some(agent) = system
        .pointer("/policy/egress/identity/user_agent")
        .and_then(Value::as_str)
    {
        policy.user_agent = agent.to_string();
    }
    policy
}
