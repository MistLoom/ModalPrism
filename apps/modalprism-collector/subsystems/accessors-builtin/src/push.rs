//! `push` 访问器。组合根把已经接受的 HTTP 请求交给 `ingest`。

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use accessors::{
    AccessContext, AccessError, AccessRequest, Accessor, AccessorCapabilities, CaptureNetwork,
};
use async_trait::async_trait;
use bytes::Bytes;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use strata_model::{Locator, RawEnvelope, Timestamp};
use tokio::sync::Mutex;

use crate::envelope::{Page, from_page};
use crate::headers::header;
use crate::ready::{config, permanent, spec_error};

pub struct Incoming {
    pub channel: String,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
}

struct Bucket {
    tokens: f64,
    updated: Instant,
}

struct Seen {
    at: Timestamp,
    hash: String,
    envelopes: Vec<RawEnvelope>,
}

pub struct PushAccessor {
    buckets: Mutex<BTreeMap<String, Bucket>>,
    seen: Mutex<BTreeMap<String, Seen>>,
}

impl Default for PushAccessor {
    fn default() -> Self {
        Self::new()
    }
}

impl PushAccessor {
    pub fn new() -> Self {
        Self {
            buckets: Mutex::new(BTreeMap::new()),
            seen: Mutex::new(BTreeMap::new()),
        }
    }

    pub async fn ingest(
        &self,
        spec: &Value,
        incoming: &Incoming,
        ctx: &AccessContext,
        policy: &Value,
        now: Timestamp,
    ) -> Result<Vec<RawEnvelope>, AccessError> {
        if ctx.is_cancelled() {
            return Err(AccessError::cancelled());
        }
        let spec = parse_spec(spec)?;
        if incoming.channel.is_empty() {
            return Err(config("locator"));
        }
        let Some(token) = presented(&incoming.headers) else {
            return Err(permanent("unauthorized"));
        };
        if !ct_eq(&token, &spec.token) {
            return Err(permanent("unauthorized"));
        }
        let hash = hex(Sha256::digest(&incoming.body));
        let map_key = header(&incoming.headers, "idempotency-key")
            .map(|key| format!("{}\n{key}", incoming.channel));
        if let Some(map_key) = map_key {
            let mut seen = self.seen.lock().await;
            seen.retain(|_, item| fresh(item.at, now));
            if let Some(found) = seen.get(&map_key) {
                if found.hash != hash {
                    return Err(permanent("idempotency"));
                }
                return Ok(found.envelopes.clone());
            }
            take_token(
                &self.buckets,
                &incoming.channel,
                spec.rate_per_sec,
                spec.burst,
            )
            .await?;
            let envelopes = one_envelope(incoming, ctx, policy)?;
            if seen.len() >= 1024 {
                if let Some(oldest) = seen
                    .iter()
                    .min_by_key(|(_, item)| item.at)
                    .map(|(key, _)| key.clone())
                {
                    seen.remove(&oldest);
                }
            }
            seen.insert(
                map_key,
                Seen {
                    at: now,
                    hash,
                    envelopes: envelopes.clone(),
                },
            );
            return Ok(envelopes);
        }
        take_token(
            &self.buckets,
            &incoming.channel,
            spec.rate_per_sec,
            spec.burst,
        )
        .await?;
        one_envelope(incoming, ctx, policy)
    }
}

fn one_envelope(
    incoming: &Incoming,
    ctx: &AccessContext,
    policy: &Value,
) -> Result<Vec<RawEnvelope>, AccessError> {
    let started = Instant::now();
    Ok(vec![from_page(
        ctx,
        policy,
        started,
        Page {
            locator: Locator::PushChannel {
                channel: incoming.channel.clone(),
            },
            final_url: format!("push://{}", incoming.channel),
            status: 200,
            headers: incoming.headers.clone(),
            body: incoming.body.clone(),
            cursor: None,
            receipt_id: None,
            render_level: None,
            engine: None,
            extra: Vec::new(),
            document: true,
        },
    )?])
}

fn fresh(at: Timestamp, now: Timestamp) -> bool {
    match at.checked_add(Duration::from_secs(24 * 60 * 60)) {
        Ok(limit) => now <= limit,
        Err(_) => true,
    }
}

fn presented(headers: &[(String, String)]) -> Option<String> {
    if let Some(value) = header(headers, "authorization") {
        if let Some(token) = value.strip_prefix("Bearer ") {
            return Some(token.to_string());
        }
    }
    header(headers, "x-push-token").map(str::to_string)
}

fn ct_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let mut diff = left.len() ^ right.len();
    let width = left.len().max(right.len());
    for index in 0..width {
        let a = left.get(index).copied().unwrap_or(0);
        let b = right.get(index).copied().unwrap_or(0);
        diff |= usize::from(a ^ b);
    }
    diff == 0
}

fn hex(bytes: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Spec {
    token: String,
    #[serde(default = "one")]
    rate_per_sec: u32,
    #[serde(default = "three")]
    burst: u32,
}

fn one() -> u32 {
    1
}

fn three() -> u32 {
    3
}

fn parse_spec(spec: &Value) -> Result<Spec, AccessError> {
    let spec: Spec = serde_json::from_value(spec.clone()).map_err(spec_error)?;
    if spec.token.is_empty() || spec.rate_per_sec == 0 || spec.burst == 0 {
        return Err(config("spec 无效"));
    }
    Ok(spec)
}

async fn take_token(
    buckets: &Mutex<BTreeMap<String, Bucket>>,
    channel: &str,
    rate: u32,
    burst: u32,
) -> Result<(), AccessError> {
    let mut buckets = buckets.lock().await;
    let bucket = buckets.entry(channel.to_string()).or_insert(Bucket {
        tokens: f64::from(burst),
        updated: Instant::now(),
    });
    let elapsed = bucket.updated.elapsed().as_secs_f64();
    bucket.updated = Instant::now();
    bucket.tokens = (bucket.tokens + elapsed * f64::from(rate)).min(f64::from(burst));
    if bucket.tokens < 1.0 {
        return Err(AccessError::new(
            strata_model::ErrorClass::Transient,
            "rate-limit",
        ));
    }
    bucket.tokens -= 1.0;
    Ok(())
}

#[async_trait]
impl Accessor for PushAccessor {
    fn id(&self) -> &str {
        "push"
    }

    fn version(&self) -> &str {
        "1"
    }

    fn capabilities(&self) -> &AccessorCapabilities {
        &CAPS
    }

    fn validate(&self, spec: &Value) -> Result<(), AccessError> {
        parse_spec(spec).map(|_| ())
    }

    async fn access(
        &self,
        _request: AccessRequest,
        _ctx: &AccessContext,
    ) -> Result<Vec<RawEnvelope>, AccessError> {
        Err(config("inbound"))
    }
}

fn caps() -> AccessorCapabilities {
    AccessorCapabilities {
        protocols: vec!["push".into()],
        conditional: false,
        render: false,
        layout: false,
        capture_network: CaptureNetwork::None,
        interact: false,
        paid: None,
        batchable: false,
        inbound: true,
    }
}

static CAPS: std::sync::LazyLock<AccessorCapabilities> = std::sync::LazyLock::new(caps);
