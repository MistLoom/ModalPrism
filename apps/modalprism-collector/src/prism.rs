//! 结构化识别引擎的宿主侧实现。
//!
//! rawprism 是仓颉运行时 Guest（fun 协议、无 logic 面），不进 surface 治理表，
//! 由组合根在 Host 就绪后注入 [`Services`]，采集链路只认 [`adapters::Prism`]。

use std::sync::Arc;

use ability_core::AbilityHost;
use ability_support::{InstanceId, MethodId};
use adapters::{AdapterError, Prism};
use async_trait::async_trait;
use frame_core::{Gateway, InvokeRequest};
use serde_json::Value;
use strata_model::ErrorClass;

pub struct HostPrism {
    host: Arc<AbilityHost>,
}

impl HostPrism {
    pub fn new(host: Arc<AbilityHost>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Prism for HostPrism {
    async fn invoke(&self, method: &str, args: Value) -> Result<Value, AdapterError> {
        let instance = InstanceId::new("rawprism")
            .map_err(|error| AdapterError::permanent(error.to_string()))?;
        let method =
            MethodId::new(method).map_err(|error| AdapterError::permanent(error.to_string()))?;
        let response = self
            .host
            .invoke(InvokeRequest::new(instance, method, args))
            .await
            .map_err(|error| AdapterError::new(ErrorClass::Transient, error.to_string()))?;
        Ok(response.data)
    }
}
