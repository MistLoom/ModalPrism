use std::sync::Arc;

use async_trait::async_trait;

use crate::{Gateway, Result};

/// 对外协议端口。将外部协议映射为统一 [`crate::InvokeRequest`] 后交给管道。
#[async_trait]
pub trait Port: Send {
    fn name(&self) -> &'static str;

    async fn serve(self: Box<Self>, gateway: Arc<dyn Gateway>) -> Result<()>;
}
