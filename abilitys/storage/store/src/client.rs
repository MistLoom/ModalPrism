//! 调用方 `use StoreExt`。结构体绑定在 Handle 上，不暴露后端。

use ability_support::{AbilityCtx, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::codec::b64;
use crate::types::{
    AcquireArgs, AcquireOut, BlobGetArgs, BlobOut, BlobPutArgs, BlobWriteArgs, GetArgs, GetOut,
    KeyArgs, ListArgs, ListOut, MergeArgs, OkOut, PutArgs, ReleaseArgs, StatOut,
};

const CHUNK: usize = 24 * 1024;

/// 从当前 ctx 解析调用方能力 id，作为默认 owner。
pub fn caller_owner(ctx: &dyn AbilityCtx) -> String {
    let inst = ctx.instance();
    ctx.catalog()
        .into_iter()
        .find(|d| &d.instance == inst)
        .map(|d| d.ability_id.to_string())
        .unwrap_or_else(|| inst.to_string())
}

#[ability_support::client(id = "store", ext = store)]
pub trait Store {
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

impl<'a> Clone for StoreHandle<'a> {
    fn clone(&self) -> Self {
        Self {
            ctx: self.ctx,
            key: self.key.clone(),
        }
    }
}

impl<'a> StoreHandle<'a> {
    fn owner(&self) -> String {
        caller_owner(self.ctx)
    }

    /// 读取并反序列化；不存在则 `T::default()`。
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

    /// 加载（或缺省）→ 修改字段 → 写回。
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

    pub async fn write_bytes(self, key: &str, bytes: &[u8]) -> Result<()> {
        if bytes.is_empty() {
            self.clone()
                .blob_put(BlobPutArgs {
                    owner: Some(self.owner()),
                    space: None,
                    key: key.into(),
                    data: String::new(),
                    append: false,
                })
                .await?;
            return Ok(());
        }
        let owner = Some(self.owner());
        let mut offset = 0u64;
        for chunk in bytes.chunks(CHUNK) {
            offset += chunk.len() as u64;
            let last = offset as usize >= bytes.len();
            self.clone()
                .blob_write(BlobWriteArgs {
                    owner: owner.clone(),
                    space: None,
                    key: key.into(),
                    offset: offset - chunk.len() as u64,
                    data: b64(chunk),
                    last,
                })
                .await?;
        }
        Ok(())
    }

    pub async fn read_bytes(self, key: &str) -> Result<Vec<u8>> {
        use crate::codec::decode_b64;
        let owner = Some(self.owner());
        let mut out = Vec::new();
        let mut offset = 0u64;
        loop {
            let chunk = self
                .clone()
                .blob_get(BlobGetArgs {
                    owner: owner.clone(),
                    space: None,
                    key: key.into(),
                    offset,
                    len: Some(CHUNK as u64),
                })
                .await?;
            if chunk.data.is_empty() {
                break;
            }
            let bytes = decode_b64(&chunk.data)?;
            offset += bytes.len() as u64;
            out.extend_from_slice(&bytes);
            if chunk.eof {
                break;
            }
        }
        Ok(out)
    }
}
