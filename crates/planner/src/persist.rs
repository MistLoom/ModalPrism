//! 节奏与健康热数据。

use std::collections::BTreeMap;

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::error::PlannerError;

#[async_trait]
pub trait Persist: Send + Sync {
    async fn load(&self, key: &str) -> Result<Option<Vec<u8>>, PlannerError>;
    async fn store(&self, key: &str, bytes: &[u8]) -> Result<(), PlannerError>;
}

#[derive(Default)]
pub struct MemoryPersist {
    rows: Mutex<BTreeMap<String, Vec<u8>>>,
}

#[async_trait]
impl Persist for MemoryPersist {
    async fn load(&self, key: &str) -> Result<Option<Vec<u8>>, PlannerError> {
        Ok(self.rows.lock().await.get(key).cloned())
    }

    async fn store(&self, key: &str, bytes: &[u8]) -> Result<(), PlannerError> {
        self.rows
            .lock()
            .await
            .insert(key.to_string(), bytes.to_vec());
        Ok(())
    }
}
