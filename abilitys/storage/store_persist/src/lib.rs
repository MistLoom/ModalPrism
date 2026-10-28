//! Ability `store.persist`：持久空间门面。

mod generated;
mod logic;

use ability_store::{
    AcquireArgs, AcquireOut, BlobGetArgs, BlobOut, BlobPutArgs, BlobWriteArgs, GetArgs, GetOut,
    KeyArgs, ListArgs, ListOut, MergeArgs, OkOut, PutArgs, ReleaseArgs, StatOut,
};
use ability_support::Result;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

pub use generated::provider::provider;

#[ability_support::client(id = "store.persist", ext = persist)]
pub trait Persist {
    async fn get(&self, args: GetArgs) -> Result<GetOut>;
    async fn put(&self, args: PutArgs) -> Result<OkOut>;
    async fn merge(&self, args: MergeArgs) -> Result<GetOut>;
    async fn delete(&self, args: KeyArgs) -> Result<OkOut>;
    async fn exists(&self, args: KeyArgs) -> Result<StatOut>;
    async fn list(&self, args: ListArgs) -> Result<ListOut>;
    async fn purge(&self, args: ListArgs) -> Result<OkOut>;
    async fn stat(&self, args: KeyArgs) -> Result<StatOut>;
    async fn blob_get(&self, args: BlobGetArgs) -> Result<BlobOut>;
    async fn blob_put(&self, args: BlobPutArgs) -> Result<OkOut>;
    async fn blob_write(&self, args: BlobWriteArgs) -> Result<OkOut>;
    async fn acquire(&self, args: AcquireArgs) -> Result<AcquireOut>;
    async fn release(&self, args: ReleaseArgs) -> Result<OkOut>;
}

impl<'a> Clone for PersistHandle<'a> {
    fn clone(&self) -> Self {
        Self {
            ctx: self.ctx,
            key: self.key.clone(),
        }
    }
}

impl<'a> PersistHandle<'a> {
    fn owner(&self) -> String {
        ability_store::caller_owner(self.ctx)
    }

    pub async fn load<T: DeserializeOwned + Default>(self, key: &str) -> Result<T> {
        let out = self
            .clone()
            .get(GetArgs {
                owner: Some(self.owner()),
                space: None,
                key: key.into(),
                codec: None,
            })
            .await?;
        match out.value {
            Some(v) => {
                serde_json::from_value(v).map_err(|e| ability_support::Error::failed(e.to_string()))
            }
            None => Ok(T::default()),
        }
    }

    pub async fn save<T: Serialize>(self, key: &str, value: &T) -> Result<()> {
        let v = serde_json::to_value(value)
            .map_err(|e| ability_support::Error::failed(e.to_string()))?;
        let owner = Some(self.owner());
        self.put(PutArgs {
            owner,
            space: None,
            key: key.into(),
            value: v,
            codec: None,
        })
        .await?;
        Ok(())
    }

    pub async fn mutate<T, F>(self, key: &str, f: F) -> Result<T>
    where
        T: Serialize + DeserializeOwned + Default,
        F: FnOnce(&mut T),
    {
        let mut doc: T = self.clone().load(key).await?;
        f(&mut doc);
        self.save(key, &doc).await?;
        Ok(doc)
    }
}
