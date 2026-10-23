//! 出站热数据的读写。调用方把字节放进控制库。

use std::collections::BTreeMap;

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::fail::store_error;

#[async_trait]
pub trait Persist: Send + Sync {
    async fn load(&self, key: &str) -> Result<Option<Vec<u8>>, accessors::AccessError>;
    async fn store(&self, key: &str, bytes: &[u8]) -> Result<(), accessors::AccessError>;
}

#[derive(Default)]
pub struct MemoryPersist {
    rows: Mutex<BTreeMap<String, Vec<u8>>>,
}

#[async_trait]
impl Persist for MemoryPersist {
    async fn load(&self, key: &str) -> Result<Option<Vec<u8>>, accessors::AccessError> {
        Ok(self.rows.lock().await.get(key).cloned())
    }

    async fn store(&self, key: &str, bytes: &[u8]) -> Result<(), accessors::AccessError> {
        self.rows
            .lock()
            .await
            .insert(key.to_string(), bytes.to_vec());
        Ok(())
    }
}

pub(crate) async fn load_db<P: Persist>(
    persist: &P,
) -> Result<crate::state::Db, accessors::AccessError> {
    let Some(bytes) = persist.load("state").await? else {
        return Ok(crate::state::Db::default());
    };
    serde_json::from_slice(&bytes).map_err(store_error)
}

pub(crate) async fn save_db<P: Persist>(
    persist: &P,
    db: &crate::state::Db,
) -> Result<(), accessors::AccessError> {
    let bytes = serde_json::to_vec(db).map_err(store_error)?;
    persist.store("state", &bytes).await
}
