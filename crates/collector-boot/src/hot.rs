use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use strata_client::Strata;

#[derive(Clone)]
pub struct Hot {
    strata: Arc<Strata>,
    kind: &'static str,
}

impl Hot {
    pub fn new(strata: Arc<Strata>, kind: &'static str) -> Self {
        Self { strata, kind }
    }
}

async fn load_bytes(hot: &Hot) -> Result<Option<Vec<u8>>, String> {
    match hot
        .strata
        .hot_get(hot.kind, "state")
        .await
        .map_err(|e| e.to_string())?
    {
        Some(value) => Ok(Some(value.to_string().into_bytes())),
        None => Ok(None),
    }
}

async fn store_bytes(hot: &Hot, bytes: &[u8]) -> Result<(), String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    hot.strata
        .hot_put(hot.kind, "state", &value)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[async_trait]
impl egress::Persist for Hot {
    async fn load(&self, key: &str) -> Result<Option<Vec<u8>>, accessors::AccessError> {
        if key != "state" {
            return Ok(None);
        }
        load_bytes(self).await.map_err(|error| {
            accessors::AccessError::with_source(
                strata_model::ErrorClass::Transient,
                "控制热数据读写失败",
                std::io::Error::other(error),
            )
        })
    }

    async fn store(&self, key: &str, bytes: &[u8]) -> Result<(), accessors::AccessError> {
        if key != "state" {
            return Err(accessors::AccessError::new(
                strata_model::ErrorClass::Transient,
                "控制热数据读写失败",
            ));
        }
        store_bytes(self, bytes).await.map_err(|error| {
            accessors::AccessError::with_source(
                strata_model::ErrorClass::Transient,
                "控制热数据读写失败",
                std::io::Error::other(error),
            )
        })
    }
}

#[async_trait]
impl paid::Persist for Hot {
    async fn load(&self, key: &str) -> Result<Option<Vec<u8>>, accessors::AccessError> {
        egress::Persist::load(self, key).await
    }

    async fn store(&self, key: &str, bytes: &[u8]) -> Result<(), accessors::AccessError> {
        egress::Persist::store(self, key, bytes).await
    }
}
