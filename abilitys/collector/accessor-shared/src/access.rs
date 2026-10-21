use std::path::PathBuf;
use std::sync::Arc;

use ability_support::{AbilityCtx, MethodId};
use async_trait::async_trait;
use bytes::Bytes;
use collector_runtime::execute::NoSession;
use collector_runtime::stage::DiskStaging;
use egress::Policy;
use paid::CallPolicy;
use serde_json::{Value, json};

use accessors::{
    AccessContext, AccessError, EgressGateway, HttpRequest, HttpResponse, PaidCall, PaidGateway,
    PaidResponse,
};

pub struct AbilityAccess;

impl AbilityAccess {
    pub fn build(
        ctx: Arc<dyn AbilityCtx>,
        egress_policy: Policy,
        module_policy: Value,
        _paid_policy: CallPolicy,
        staging_dir: PathBuf,
    ) -> AccessContext {
        let egress_ctx = Arc::clone(&ctx);
        let paid_ctx = Arc::clone(&ctx);
        let _ = egress_policy;
        AccessContext {
            egress: Arc::new(CtxEgress {
                ctx: egress_ctx,
                module_policy,
            }),
            paid: Arc::new(CtxPaid { ctx: paid_ctx }),
            sessions: Arc::new(NoSession),
            staging: Arc::new(DiskStaging { dir: staging_dir }),
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

struct CtxEgress {
    ctx: Arc<dyn AbilityCtx>,
    module_policy: Value,
}

#[async_trait]
impl EgressGateway for CtxEgress {
    async fn request(&self, request: HttpRequest) -> Result<HttpResponse, AccessError> {
        let method = MethodId::new("http_request").expect("method id");
        let value = self
            .ctx
            .call(
                "egress",
                &method,
                json!({
                    "method": request.method,
                    "url": request.url,
                    "headers": request.headers,
                    "body": request.body.to_vec(),
                    "modulePolicy": self.module_policy,
                }),
            )
            .await
            .map_err(|e| AccessError::new(strata_model::ErrorClass::Transient, e.to_string()))?;
        Ok(HttpResponse {
            status: value
                .get("status")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u16,
            headers: value
                .get("headers")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default(),
            body: value
                .get("body")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    Bytes::from(
                        arr.iter()
                            .filter_map(|v| v.as_u64().map(|n| n as u8))
                            .collect::<Vec<u8>>(),
                    )
                })
                .unwrap_or_default(),
            final_url: value
                .get("finalUrl")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        })
    }
}

struct CtxPaid {
    ctx: Arc<dyn AbilityCtx>,
}

#[async_trait]
impl PaidGateway for CtxPaid {
    async fn call(&self, request: PaidCall) -> Result<PaidResponse, AccessError> {
        let method = MethodId::new("call").expect("method id");
        let value = self
            .ctx
            .call(
                "paid_gateway",
                &method,
                json!({
                    "endpoint": request.endpoint,
                    "headers": request.headers,
                    "body": request.body.to_vec(),
                    "params": request.params,
                }),
            )
            .await
            .map_err(|e| AccessError::new(strata_model::ErrorClass::Transient, e.to_string()))?;
        Ok(PaidResponse {
            status: value
                .get("status")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u16,
            headers: value
                .get("headers")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default(),
            body: value
                .get("body")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    Bytes::from(
                        arr.iter()
                            .filter_map(|v| v.as_u64().map(|n| n as u8))
                            .collect::<Vec<u8>>(),
                    )
                })
                .unwrap_or_default(),
            receipt_id: value
                .get("receiptId")
                .and_then(Value::as_str)
                .and_then(|text| strata_model::ReceiptId::new(text).ok()),
        })
    }
}
