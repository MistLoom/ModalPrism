use std::sync::Arc;

use async_trait::async_trait;
use collector_gateway::{EGRESS_POLICY, HttpTransport, SystemResolver};
use egress::{Policy, TcpDial};
use serde_json::Value;
use url::Url;

use accessors::{AccessError, HttpRequest, PaidCall, PaidResponse};
use paid::PaidTransport;

use crate::hot::Hot;

pub type EgressGw = egress::Gateway<Hot, HttpTransport, SystemResolver, TcpDial>;
pub type PaidGw = paid::Gateway<Hot, PaidBridge>;

pub struct PaidBridge {
    pub egress: Arc<EgressGw>,
    pub fallback: Policy,
}

#[async_trait]
impl PaidTransport for PaidBridge {
    async fn send(&self, call: PaidCall) -> Result<PaidResponse, AccessError> {
        let Ok(mut url) = Url::parse(&call.endpoint) else {
            return Err(AccessError::new(strata_model::ErrorClass::Permanent, "scheme"));
        };
        if url.scheme() != "http" && url.scheme() != "https" {
            return Err(AccessError::new(strata_model::ErrorClass::Permanent, "scheme"));
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
