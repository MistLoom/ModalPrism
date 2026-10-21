use std::sync::Arc;
use ability_support::{AbilityCtx, MethodId};
use accessors::{
    AccessContext, AccessError, AccessRequest, Accessor, AccessorCapabilities, CaptureNetwork,
};
use strata_model::RawEnvelope;
use async_trait::async_trait;
use serde_json::{Value, json};
use strata_model::ErrorClass;

pub struct CtxAccessor {
    ctx: Arc<dyn AbilityCtx>,
    instance: String,
    id: String,
    version: String,
    capabilities: AccessorCapabilities,
}

impl CtxAccessor {
    pub fn open(ctx: Arc<dyn AbilityCtx>, implementation: &str) -> Result<Self, AccessError> {
        let name = implementation
            .strip_prefix("builtin:")
            .ok_or_else(|| config_error("implementation"))?;
        let instance = match name {
            "http" => "accessor_http",
            "rss" => "accessor_rss",
            "api" => "accessor_api",
            "paid-api" => "accessor_paid_api",
            "search" => "accessor_search",
            "push" => "accessor_push",
            "file" => "accessor_file",
            "browser" => "accessor_browser",
            _ => return Err(config_error("implementation")),
        };
        Ok(Self {
            ctx,
            instance: instance.to_string(),
            id: instance.to_string(),
            version: "1".into(),
            capabilities: AccessorCapabilities {
                protocols: Vec::new(),
                conditional: false,
                render: false,
                layout: false,
                capture_network: CaptureNetwork::None,
                interact: false,
                paid: None,
                batchable: false,
                inbound: false,
            },
        })
    }
}

fn config_error(message: impl Into<String>) -> AccessError {
    AccessError::new(ErrorClass::Config, message)
}

#[async_trait]
impl Accessor for CtxAccessor {
    fn id(&self) -> &str {
        &self.id
    }

    fn version(&self) -> &str {
        &self.version
    }

    fn capabilities(&self) -> &AccessorCapabilities {
        &self.capabilities
    }

    fn validate(&self, _spec: &Value) -> Result<(), AccessError> {
        Ok(())
    }

    async fn access(
        &self,
        request: AccessRequest,
        _ctx: &AccessContext,
    ) -> Result<Vec<RawEnvelope>, AccessError> {
        let method = MethodId::new("access").expect("method id");
        let value = self
            .ctx
            .call(
                &self.instance,
                &method,
                json!({
                    "taskId": request.task_id.to_string(),
                    "sourceId": request.source_id.to_string(),
                    "locator": request.locator,
                    "spec": request.spec,
                    "cursor": request.cursor,
                    "policy": request.policy,
                }),
            )
            .await
            .map_err(|e| AccessError::new(ErrorClass::Transient, e.to_string()))?;
        serde_json::from_value(value).map_err(|e| config_error(e.to_string()))
    }
}
