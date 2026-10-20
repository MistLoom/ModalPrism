//! 搜索适配器、内容适配器和检测器。这些 trait 没有写入方法。

use async_trait::async_trait;
use serde_json::Value;

use crate::error::AdapterError;
use crate::model::{
    AdapterInput, AdapterMeta, ContentOutput, Declared, Detection, DiscoverOutput, Probe,
    ReadonlyView, RegionHit,
};

#[async_trait]
pub trait SearchAdapter: Send + Sync {
    fn meta(&self) -> &AdapterMeta;
    fn validate(&self, spec: &Value) -> Result<(), AdapterError> {
        let _ = spec;
        Ok(())
    }
    async fn probe(&self, input: &AdapterInput, spec: &Value) -> Option<Probe> {
        let _ = (input, spec);
        None
    }
    async fn discover(
        &self,
        input: &AdapterInput,
        spec: &Value,
    ) -> Result<DiscoverOutput, AdapterError>;
}

#[async_trait]
pub trait ContentAdapter: Send + Sync {
    fn meta(&self) -> &AdapterMeta;
    fn validate(&self, spec: &Value) -> Result<(), AdapterError> {
        let _ = spec;
        Ok(())
    }
    async fn probe(&self, input: &AdapterInput, spec: &Value) -> Option<Probe> {
        let _ = (input, spec);
        None
    }
    async fn adapt(
        &self,
        input: &AdapterInput,
        spec: &Value,
    ) -> Result<ContentOutput, AdapterError>;
}

#[async_trait]
pub trait Detector: Send + Sync {
    fn id(&self) -> &str;
    fn version(&self) -> &str;
    async fn detect(
        &self,
        view: &ReadonlyView,
        declared: &Declared,
    ) -> Result<Detection, AdapterError>;
    async fn regions(&self, view: &ReadonlyView, wanted: &[String]) -> Vec<RegionHit> {
        let _ = (view, wanted);
        Vec::new()
    }
}
