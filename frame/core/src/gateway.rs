use async_trait::async_trait;

use crate::{CatalogEntry, InstanceId, InvokeRequest, InvokeResponse, MethodId, Result};

/// 调用管道门面。端口与 Fun 只能通过本 trait 进入。
#[async_trait]
pub trait Gateway: Send + Sync {
    async fn invoke(&self, request: InvokeRequest) -> Result<InvokeResponse>;

    fn catalog(&self) -> Vec<CatalogEntry>;

    fn get_entry(&self, instance: &InstanceId, method: &MethodId) -> Option<CatalogEntry>;
}
