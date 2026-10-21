//! 出站传输。地址由网关在 SSRF 检查之后给出。

use std::net::IpAddr;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use accessors::EgressGateway;
use async_trait::async_trait;
use egress::{
    CrawlDelayMode, Exempt, Policy, PrivateNetworks, Resolver, RobotsMode, TcpDial, Transport,
    Unreachable,
};
use paid::{CallPolicy, PaidTransport, UnknownMode};
use serde_json::Value;
use strata_model::ErrorClass;
use url::Url;

use accessors::{AccessError, HttpRequest, HttpResponse, PaidCall, PaidResponse};

use strata_shared::persist::Hot;
use tokio::net::TcpListener;

pub struct HttpTransport;

pub struct SystemResolver;

pub struct PaidBridge {
    pub egress: Arc<EgressGw>,
    pub fallback: Policy,
}

tokio::task_local! {
    pub static EGRESS_POLICY: Policy;
    pub static REQUEST_DEADLINE: Instant;
}

pub type EgressGw = egress::Gateway<Hot, HttpTransport, SystemResolver, TcpDial>;
pub type PaidGw = paid::Gateway<Hot, PaidBridge>;

fn permanent(message: &str) -> AccessError {
    AccessError::new(ErrorClass::Permanent, message)
}

/// 本次任务剩余到截止时间的时长。没有任务截止时用 30 秒。
fn request_budget() -> Result<Duration, AccessError> {
    let budget = REQUEST_DEADLINE
        .try_with(|deadline| deadline.saturating_duration_since(Instant::now()))
        .unwrap_or(Duration::from_secs(30));
    if budget.is_zero() {
        Err(AccessError::new(ErrorClass::Transient, "deadline"))
    } else {
        Ok(budget)
    }
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
        let budget = request_budget()?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(budget)
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
        let policy = EGRESS_POLICY
            .try_with(|policy| policy.clone())
            .unwrap_or_else(|_| self.fallback.clone());
        let response = self
            .egress
            .request_governed(
                &policy,
                HttpRequest {
                    method,
                    url: url.to_string(),
                    headers: call.headers.clone(),
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
    policy_from(system, &Value::Null)
}

pub fn policy_from(system: &Value, module_policy: &Value) -> Policy {
    let effective = config::overlay_system_policy(system, module_policy);
    let mut policy = Policy::library("ModalPrism");
    if effective
        .pointer("/egress/private_networks")
        .and_then(Value::as_str)
        == Some("allow")
    {
        policy.private_networks = PrivateNetworks::Allow;
    }
    if let Some(agent) = effective
        .pointer("/egress/identity/user_agent")
        .and_then(Value::as_str)
    {
        policy.user_agent = agent.to_string();
    }
    if let Some(contact) = effective
        .pointer("/egress/identity/contact")
        .and_then(Value::as_str)
    {
        policy.contact = Some(contact.to_string());
    }
    if let Some(mode) = effective.pointer("/robots/mode").and_then(Value::as_str) {
        policy.robots = match mode {
            "ignore" => RobotsMode::Ignore,
            "obey-with-exemptions" => RobotsMode::ObeyWithExemptions,
            _ => RobotsMode::Obey,
        };
    }
    if let Some(ttl) = effective
        .pointer("/robots/cache_ttl")
        .and_then(Value::as_str)
        .and_then(parse_duration)
    {
        policy.robots_ttl = ttl;
    }
    if let Some(mode) = effective
        .pointer("/robots/on_unreachable")
        .and_then(Value::as_str)
    {
        policy.on_unreachable = match mode {
            "allow" => Unreachable::Allow,
            "disallow" => Unreachable::Disallow,
            _ => Unreachable::Rfc9309,
        };
    }
    if let Some(mode) = effective
        .pointer("/robots/crawl_delay")
        .and_then(Value::as_str)
    {
        policy.crawl_delay = if mode == "honor" {
            CrawlDelayMode::Honor
        } else if mode == "ignore" {
            CrawlDelayMode::Ignore
        } else if let Some(rest) = mode.strip_prefix("cap:") {
            parse_duration(rest)
                .map(CrawlDelayMode::Cap)
                .unwrap_or(CrawlDelayMode::Ignore)
        } else {
            CrawlDelayMode::Ignore
        };
    }
    if let (Some(category), Some(reason)) = (
        effective
            .pointer("/robots/exempt_as")
            .and_then(Value::as_str),
        effective
            .pointer("/robots/robots_reason")
            .and_then(Value::as_str),
    ) {
        policy.exempt = Some(Exempt {
            category: category.to_string(),
            reason: reason.to_string(),
        });
    }
    policy
}

pub fn call_policy_from(system: &Value, module_policy: &Value) -> CallPolicy {
    let effective = config::overlay_system_policy(system, module_policy);
    let mut policy = CallPolicy::default();
    if effective
        .pointer("/paid/unknown_receipt")
        .and_then(Value::as_str)
        == Some("manual")
    {
        policy.unknown = UnknownMode::Manual;
    }
    policy
}

fn parse_duration(text: &str) -> Option<Duration> {
    let split = text.find(|ch: char| !ch.is_ascii_digit())?;
    let count: u64 = text[..split].parse().ok()?;
    let scale = match &text[split..] {
        "ms" => 1,
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return None,
    };
    Some(Duration::from_millis(count.saturating_mul(scale)))
}

pub struct BoundEgress {
    pub inner: Arc<EgressGw>,
    pub policy: Policy,
}

#[async_trait]
impl EgressGateway for BoundEgress {
    async fn request(&self, request: HttpRequest) -> Result<HttpResponse, AccessError> {
        self.inner.request_governed(&self.policy, request).await
    }
}

pub struct BoundPaid {
    pub inner: Arc<PaidGw>,
    pub policy: CallPolicy,
}

#[async_trait]
impl accessors::PaidGateway for BoundPaid {
    async fn call(&self, request: PaidCall) -> Result<PaidResponse, AccessError> {
        self.inner.call_governed(&self.policy, request).await
    }
}

pub async fn serve_proxy<F>(gateway: Arc<EgressGw>, listener: TcpListener, policy_of: F)
where
    F: Fn() -> Policy,
{
    egress::serve_each(gateway.as_ref(), listener, policy_of).await;
}
