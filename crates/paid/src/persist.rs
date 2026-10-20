//! 付费热数据的读写。

use std::collections::BTreeMap;

use async_trait::async_trait;
use tokio::sync::Mutex;

use accessors::AccessError;
use strata_model::ErrorClass;

#[async_trait]
pub trait Persist: Send + Sync {
    async fn load(&self, key: &str) -> Result<Option<Vec<u8>>, AccessError>;
    async fn store(&self, key: &str, bytes: &[u8]) -> Result<(), AccessError>;
}

#[derive(Default)]
pub struct MemoryPersist {
    rows: Mutex<BTreeMap<String, Vec<u8>>>,
}

#[async_trait]
impl Persist for MemoryPersist {
    async fn load(&self, key: &str) -> Result<Option<Vec<u8>>, AccessError> {
        Ok(self.rows.lock().await.get(key).cloned())
    }

    async fn store(&self, key: &str, bytes: &[u8]) -> Result<(), AccessError> {
        self.rows
            .lock()
            .await
            .insert(key.to_string(), bytes.to_vec());
        Ok(())
    }
}

pub(crate) fn store_error(source: impl std::error::Error + Send + Sync + 'static) -> AccessError {
    AccessError::with_source(ErrorClass::Transient, "控制热数据读写失败", source)
}
